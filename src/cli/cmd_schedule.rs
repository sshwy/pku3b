use crate::api;
use crate::cli::pbar;
use crate::config;
use crate::utils;
use anyhow::Context;
use compio::buf::buf_try;
use compio::fs;
use compio::io::AsyncWriteExt;
use std::io::Write;

/// 获取个人课表
pub async fn list(force: bool, raw: bool) -> anyhow::Result<()> {
    let client = if force {
        api::Client::new_nocache()
    } else {
        api::Client::default()
    };

    let sp = pbar::new_spinner();
    sp.set_message("reading config...");
    let cfg_path = utils::default_config_path();
    let cfg = config::read_cfg(&cfg_path)
        .await
        .context("read config file")?;

    sp.set_message("正在登录门户...");

    let portal = client
        .portal(&cfg.username, &cfg.password)
        .await
        .context("登录门户失败")?;

    sp.set_message("正在获取课表...");

    let raw_data = portal.get_my_course_table().await?;

    sp.finish_and_clear();

    // 输出结果
    let mut outbuf = Vec::new();
    if raw {
        writeln!(outbuf, "{}", raw_data)?;
    } else {
        // 解析个人课表
        let json: serde_json::Value = serde_json::from_str(&raw_data)?;
        if let Some(courses) = json.get("course").and_then(|c| c.as_array()) {
            if courses.is_empty() {
                writeln!(outbuf, "暂无课表数据")?;
            } else {
                writeln!(outbuf, "📅 个人课表\n")?;

                // 按周几分组显示
                let days = [
                    ("mon", "周一"),
                    ("tue", "周二"),
                    ("wed", "周三"),
                    ("thu", "周四"),
                    ("fri", "周五"),
                    ("sat", "周六"),
                    ("sun", "周日"),
                ];

                for (day_key, day_name) in days.iter() {
                    // 收集该天的所有课程
                    let mut day_slots: Vec<(usize, String)> = Vec::new();

                    for (idx, slot) in courses.iter().enumerate() {
                        let slot_num = idx + 1; // 第几节

                        if let Some(course) = slot.get(day_key) {
                            if let Some(name) = course.get("courseName").and_then(|n| n.as_str()) {
                                if !name.is_empty() {
                                    let clean_info = format_course_info(name);
                                    day_slots.push((slot_num, clean_info));
                                }
                            }
                        }
                    }

                    if !day_slots.is_empty() {
                        writeln!(outbuf, "【{}】", day_name)?;

                        // 合并连续节次
                        let mut i = 0;
                        while i < day_slots.len() {
                            let (start_slot, info) = &day_slots[i];
                            let mut end_slot = *start_slot;

                            // 检查后续是否有相同课程
                            let mut j = i + 1;
                            while j < day_slots.len() && day_slots[j].1 == *info {
                                end_slot = day_slots[j].0;
                                j += 1;
                            }

                            // 输出
                            if start_slot == &end_slot {
                                writeln!(outbuf, "  第{}节: {}", start_slot, info)?;
                            } else {
                                writeln!(outbuf, "  第{}-{}节: {}", start_slot, end_slot, info)?;
                            }

                            i = j;
                        }

                        writeln!(outbuf)?;
                    }
                }
            }
        } else {
            writeln!(outbuf, "{}", raw_data)?;
        }
    }
    buf_try!(@try fs::stdout().write_all(outbuf).await);

    Ok(())
}

fn format_course_info(info: &str) -> String {
    // 提取课程名称
    let course_name = info.split("(主)").next().unwrap_or(info).trim();

    // 提取上课信息
    let mut result = course_name.to_string();

    if let Some(class_idx) = info.find("上课信息：") {
        let class_start = class_idx + 15; // "上课信息：" 是5个中文字符 = 15字节
        let rest = &info[class_start..];
        let class_end = rest.find("教师：").unwrap_or(rest.len());
        let class_info = rest[..class_end].trim();
        if !class_info.is_empty() {
            result.push_str(" | ");
            result.push_str(class_info);
        }

        // 提取教师
        if let Some(teacher_idx) = rest.find("教师：") {
            let teacher_start = teacher_idx + 9; // "教师：" 是3个中文字符 = 9字节
            let teacher_rest = &rest[teacher_start..];
            let teacher_end = teacher_rest
                .find(' ')
                .or_else(|| teacher_rest.find("\u{003c}"))
                .unwrap_or(teacher_rest.len());
            let teacher = teacher_rest[..teacher_end].trim();
            if !teacher.is_empty() {
                result.push_str(" | 教师：");
                result.push_str(teacher);
            }
        }
    }

    // 提取考试信息
    if let Some(exam_idx) = info.find("考试信息：") {
        let exam_start = exam_idx + 15; // "考试信息：" 是5个中文字符 = 15字节
        let rest = &info[exam_start..];
        let exam_end = rest.find("\u{003c}").unwrap_or(rest.len());
        let exam_info = rest[..exam_end].trim();
        if !exam_info.is_empty() {
            result.push_str(" | 考试：");
            result.push_str(exam_info);
        }
    }

    result
}
