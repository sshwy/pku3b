use super::*;

use crate::api::blackboard::{AttemptFileInfo, CourseGroup, CourseMembership};
use anyhow::Context as _;
use compio::fs;
use std::io::Write as _;

#[derive(clap::Args)]
pub struct CommandTa {
    #[command(subcommand)]
    command: TaCommands,
}

#[derive(Subcommand)]
enum TaCommands {
    /// 作业查看/下载
    Hw {
        #[command(subcommand)]
        command: TaHwCommands,
    },
    /// 批改组查看
    Group {
        #[command(subcommand)]
        command: TaGroupCommands,
    },
}

#[derive(Subcommand)]
enum TaHwCommands {
    /// 列出作业（含未评分数）
    Ls {
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "false")]
        all_term: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        /// 按批改组筛选统计
        #[arg(short, long)]
        group: Option<usize>,
        /// 课程 ID（如 _98207_1），不填则交互选择
        #[arg(short = 'c', long)]
        course: Option<String>,
    },
    /// 登分（给作业打分）
    Grade {
        /// 作业编号（不填则交互选择）
        #[arg(short = 'i', long = "id")]
        id: Option<usize>,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        #[arg(short, long)]
        group: Option<usize>,
        /// 下载/评分全部历史提交（默认只取最新一次）
        #[arg(short = 'A', long, default_value = "false")]
        all_attempts: bool,
        /// 直接给指定学生打分（学号，如 _170599_1），跳过交互
        #[arg(short = 's', long, value_name = "USER_ID")]
        student: Option<String>,
        /// 分数（与 -s 配合使用）
        #[arg(short = 'S', long, value_name = "SCORE", allow_hyphen_values = true)]
        score: Option<f64>,
        /// 课程 ID（如 _98207_1），不填则交互选择
        #[arg(short = 'c', long)]
        course: Option<String>,
    },
    /// 下载作业提交文件
    Down {
        /// 作业编号（不填则交互选择）
        #[arg(short = 'i', long = "id")]
        id: Option<usize>,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "false")]
        all_term: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        /// 批改组编号（-s 覆盖此选项）
        #[arg(short, long)]
        group: Option<usize>,
        /// 只下载指定学生的提交（覆盖 -g）
        #[arg(short = 's', long, value_name = "USER_ID")]
        student: Option<String>,
        /// 下载全部（含已评分），默认仅未评分
        #[arg(short = 'a', long, default_value = "false")]
        all: bool,
        /// 下载所有作业的未评分提交（不弹出选择）
        #[arg(long, default_value = "false")]
        all_hw: bool,
        /// 不重命名文件（默认重命名为 学号_姓名_原始文件名）
        #[arg(long, default_value = "false")]
        no_rename: bool,
        /// 下载/评分全部历史提交（默认只取最新一次）
        #[arg(short = 'A', long, default_value = "false")]
        all_attempts: bool,
        /// 课程 ID（如 _98207_1），不填则交互选择
        #[arg(short = 'c', long)]
        course: Option<String>,
    },
}

#[derive(Subcommand)]
enum TaGroupCommands {
    /// 列出批改组
    Ls {
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        /// 课程 ID（如 _98207_1），不填则交互选择
        #[arg(short = 'c', long)]
        course: Option<String>,
    },
    /// 查看组成员
    Show {
        id: usize,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        /// 课程 ID（如 _98207_1），不填则交互选择
        #[arg(short = 'c', long)]
        course: Option<String>,
    },
}

pub async fn run(cmd: CommandTa, ctx: &CommandCtx<'_>) -> anyhow::Result<()> {
    match cmd.command {
        TaCommands::Hw { command } => match command {
            TaHwCommands::Ls {
                force,
                all_term,
                otp_code,
                group,
                course,
            } => ta_hw_ls(ctx, force, all_term, otp_code, group, course).await?,
            TaHwCommands::Grade {
                id,
                force,
                otp_code,
                group,
                all_attempts,
                student,
                score,
                course,
            } => {
                ta_grade(
                    ctx,
                    id,
                    force,
                    otp_code,
                    group,
                    all_attempts,
                    student,
                    score,
                    course,
                )
                .await?
            }
            TaHwCommands::Down {
                id,
                force,
                all_term,
                otp_code,
                group,
                student,
                all,
                all_hw,
                no_rename,
                all_attempts,
                course,
            } => {
                ta_hw_down(
                    ctx,
                    id,
                    force,
                    all_term,
                    otp_code,
                    group,
                    student,
                    all,
                    all_hw,
                    no_rename,
                    all_attempts,
                    course,
                )
                .await?
            }
        },
        TaCommands::Group { command } => match command {
            TaGroupCommands::Ls {
                force,
                otp_code,
                course,
            } => ta_group_ls(ctx, force, otp_code, course).await?,
            TaGroupCommands::Show {
                id,
                force,
                otp_code,
                course,
            } => ta_group_show(ctx, id, force, otp_code, course).await?,
        },
    }
    Ok(())
}

// ── Group commands ──

async fn ta_group_ls(
    ctx: &CommandCtx<'_>,
    force: bool,
    otp_code: String,
    course: Option<String>,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, course)?;

    sp.set_message("fetching groups...");
    let groups = b.get_course_groups(&course_id).await?;

    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();

    sp.finish_with_message("done.");

    let mut outbuf = Vec::new();
    writeln!(outbuf, "{D}>{D:#} {B}批改组列表{B:#} {D}<{D:#}\n")?;

    if sub_groups.is_empty() {
        writeln!(outbuf, "  未找到批改组")?;
    } else {
        for (i, g) in sub_groups.iter().enumerate() {
            let count = b
                .get_group_users(&course_id, &g.id)
                .await
                .map(|u| u.len())
                .unwrap_or(0);
            writeln!(
                outbuf,
                "  {B}{}{B:#}  {}  {D}({}人){D:#}",
                i + 1,
                g.name,
                count
            )?;
        }
    }

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

async fn ta_group_show(
    ctx: &CommandCtx<'_>,
    id: usize,
    force: bool,
    otp_code: String,
    course: Option<String>,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, course)?;

    sp.set_message("fetching groups...");
    let groups = b.get_course_groups(&course_id).await?;
    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();

    let group = sub_groups
        .get(id.wrapping_sub(1))
        .context("invalid group index")?;

    let members = b.get_group_users(&course_id, &group.id).await?;

    sp.set_message("fetching member names...");
    let mut entries = Vec::new();
    for uid in &members {
        match b.get_user_name(uid).await {
            Ok(name) => entries.push((uid.clone(), name)),
            Err(_) => entries.push((uid.clone(), "?".to_string())),
        }
    }

    sp.finish_with_message("done.");

    let mut outbuf = Vec::new();
    writeln!(
        outbuf,
        "{D}>{D:#} {B}{}{B:#} {D}({}人)<{D:#}\n",
        group.name,
        members.len()
    )?;

    for (i, (uid, name)) in entries.iter().enumerate() {
        writeln!(outbuf, "  {B}{}{B:#}  {}  {D}{}{D:#}", i + 1, name, uid)?;
    }

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

// ── HW commands ──

struct HwItem {
    name: String,
    needs_grading: usize,
    total: usize,
}

async fn ta_hw_ls(
    ctx: &CommandCtx<'_>,
    force: bool,
    _all_term: bool,
    otp_code: String,
    group_idx: Option<usize>,
    course: Option<String>,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, course)?;

    let group_users: Option<std::collections::HashSet<String>> = if let Some(gid) = group_idx {
        let groups = b.get_course_groups(&course_id).await?;
        let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();
        let group = sub_groups
            .get(gid.wrapping_sub(1))
            .context("invalid group index")?;
        let members = b.get_group_users(&course_id, &group.id).await?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    sp.set_message("fetching assignments...");
    let detail = b.course_detail(&course_id).await?;
    let columns = detail.gradebook_columns().await?;

    let mut hw_items = Vec::new();
    let mut total_ungraded = 0usize;

    for col in &columns {
        let Some(grading) = &col.grading else {
            continue;
        };
        if grading.grading_type != "Attempts" {
            continue;
        }

        let attempts = match detail.get_attempts(&col.id).await {
            Ok(a) => a,
            Err(e) => {
                log::warn!("failed to fetch attempts for {}: {e:#}", col.name);
                continue;
            }
        };

        // Users who actually submitted (have at least one attempt)
        let submitted: std::collections::HashSet<String> =
            attempts.iter().map(|a| a.user_id.clone()).collect();

        let grade_data = match detail.gradedata(&col.id).await {
            Ok(a) => a,
            Err(e) => {
                log::warn!("failed to fetch grade data for {}: {e:#}", col.name);
                continue;
            }
        };

        // Filter: in group AND has submitted AND (NeedsGrading + not exempt)
        let items: Vec<_> = if let Some(ref group_set) = group_users {
            grade_data
                .into_iter()
                .filter(|g| group_set.contains(&g.user_id) && submitted.contains(&g.user_id))
                .collect()
        } else {
            grade_data
                .into_iter()
                .filter(|g| submitted.contains(&g.user_id))
                .collect()
        };

        let needs_grading = items
            .iter()
            .filter(|g| g.status.as_deref() == Some("NeedsGrading") && !g.exempt)
            .count();
        total_ungraded += needs_grading;

        hw_items.push(HwItem {
            name: col.name.clone(),
            needs_grading,
            total: items.len(),
        });
    }

    sp.finish_with_message("done.");

    let mut outbuf = Vec::new();
    writeln!(outbuf, "{D}>{D:#} {B}作业列表{B:#} {D}<{D:#}\n")?;

    let name_max = hw_items
        .iter()
        .map(|h| h.name.chars().count())
        .max()
        .unwrap_or(20);
    for (i, hw) in hw_items.iter().enumerate() {
        let pad = " ".repeat(name_max.saturating_sub(hw.name.chars().count()));
        write!(outbuf, "  {B}{}{B:#}  {}{pad}  ", i + 1, hw.name)?;
        if hw.needs_grading > 0 {
            writeln!(
                outbuf,
                "{D}total:{} {RD}[未评: {}]{D:#}",
                hw.total, hw.needs_grading
            )?;
        } else {
            writeln!(outbuf, "{D}total:{}{D:#}", hw.total)?;
        }
    }

    writeln!(outbuf, "\n{D}合计: {RD}{} 项未评分{D:#}", total_ungraded)?;

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

async fn ta_hw_down(
    ctx: &CommandCtx<'_>,
    hw_id: Option<usize>,
    force: bool,
    _all_term: bool,
    otp_code: String,
    group_idx: Option<usize>,
    student: Option<String>,
    all: bool,
    all_hw: bool,
    no_rename: bool,
    all_attempts: bool,
    course: Option<String>,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, course)?;

    // Resolve group (skipped when -s is given without -g)
    sp.set_message("fetching groups...");
    let groups = b.get_course_groups(&course_id).await?;
    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();
    let group_name: String;
    let group_members: std::collections::HashSet<String>;
    if student.is_some() && group_idx.is_none() {
        group_name = student.clone().unwrap();
        group_members = std::collections::HashSet::new();
    } else {
        let group = resolve_group(&sub_groups, group_idx)?;
        group_name = group.name.clone();
        sp.set_message(format!("fetching members of {}...", group_name));
        group_members = b
            .get_group_users(&course_id, &group.id)
            .await?
            .into_iter()
            .collect()
    };

    // Resolve assignment
    let detail = b.course_detail(&course_id).await?;
    let columns = detail.gradebook_columns().await?;
    let hw_cols = filter_hw_columns(&columns);

    if hw_cols.is_empty() {
        anyhow::bail!("no assignment columns found");
    }

    // Get membership map (shared across all HWs)
    sp.set_message("fetching membership data...");
    let memberships = b.get_course_memberships(&course_id).await?;
    let membership_map: std::collections::HashMap<String, &CourseMembership> =
        memberships.iter().map(|m| (m.user_id.clone(), m)).collect();

    // When all_hw, iterate all assignments
    let target_hw: Vec<&crate::api::blackboard::GradebookColumn> = if all_hw {
        sp.finish_and_clear();
        let confirmed = inquire::Confirm::new(&format!(
            "确认下载 {B}全部 {hw_count} 个{B:#} 作业的未评分提交？",
            hw_count = hw_cols.len()
        ))
        .with_default(false)
        .prompt()?;
        if !confirmed {
            return Ok(());
        }
        hw_cols.iter().copied().collect()
    } else {
        vec![resolve_hw(&hw_cols, hw_id)?]
    };

    let dir = dirs::BaseDirs::new()
        .map(|d| d.home_dir().join("Downloads"))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    if !dir.exists() {
        fs::create_dir_all(&dir).await?;
    }

    let mp = pbar::new_spinner_on(ctx.multi);
    sp.finish_and_clear();

    for hw_col in target_hw {
        log::info!("fetching submissions for {}...", hw_col.name);
        let mut attempts = detail.get_attempts(&hw_col.id).await?;

        // Filter: student (-s) overrides group
        if let Some(ref sid) = student {
            attempts.retain(|a| &a.user_id == sid);
        } else {
            attempts.retain(|a| group_members.contains(&a.user_id));
        }

        // Default to ungraded only; use -a to include already-graded submissions
        if !all {
            attempts.retain(|a| a.status.as_deref() == Some("NeedsGrading") && !a.exempt);
        }

        if attempts.is_empty() {
            log::info!("{}: no matching submissions, skip", hw_col.name);
            continue;
        }

        // Deduplicate: keep only latest per user if configured
        if !all_attempts {
            attempts.sort_by(|a, b| b.attempt_date.cmp(&a.attempt_date));
            let mut seen = std::collections::HashSet::new();
            attempts.retain(|a| seen.insert(a.user_id.clone()));
        }

        let total = attempts.len();

        writeln!(
            std::io::stdout(),
            "{D}>{D:#} {B}下载 {}{B:#} - {}{D} {total} submissions{D:#}",
            group_name,
            hw_col.name,
        )?;

        let mut downloaded = 0usize;
        for (i, attempt) in attempts.iter().enumerate() {
            mp.set_message(format!(
                "[{}/{}] processing {}...",
                i + 1,
                total,
                &attempt.user_id
            ));

            let membership = match membership_map.get(&attempt.user_id) {
                Some(m) => m,
                None => {
                    log::warn!("no membership for user {}", attempt.user_id);
                    continue;
                }
            };

            let file_info = b
                .get_attempt_file_info(&attempt.id, &course_id, &hw_col.id, &membership.id)
                .await?;

            let (download_url, original_name) = match file_info {
                AttemptFileInfo::File {
                    download_url,
                    file_name,
                } => (download_url, file_name),
                AttemptFileInfo::NoFile => {
                    log::info!("{}: no file submission", attempt.user_id);
                    continue;
                }
            };

            let dest_name = if !no_rename {
                let user_name = b.get_user_name(&attempt.user_id).await.unwrap_or_default();
                if user_name.is_empty() {
                    original_name
                } else {
                    format!("{}_{}_{}", user_name, hw_col.name, original_name)
                }
            } else {
                original_name
            };
            let dest_path = dir.join(&dest_name);

            mp.set_message(format!(
                "[{}/{}] downloading {}...",
                i + 1,
                total,
                &dest_name
            ));

            match b.download_attempt_file(&download_url).await {
                Ok(data) => {
                    let r = compio::fs::write(&dest_path, data).await;
                    if let Err(e) = r.0 {
                        log::error!("failed to write {}: {e}", dest_path.display());
                    } else {
                        downloaded += 1;
                        log::info!("downloaded: {}", dest_path.display());
                    }
                }
                Err(e) => {
                    log::error!("download failed for {}: {e:#}", attempt.user_id);
                }
            }

            // Rate limit
            compio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        writeln!(
            std::io::stdout(),
            "{D}>{D:#} {GR}下载完成: {downloaded}/{total} 个文件{GR:#}{D}<{D:#}"
        )?;
    }

    Ok(())
}

async fn ta_grade(
    ctx: &CommandCtx<'_>,
    hw_id: Option<usize>,
    force: bool,
    otp_code: String,
    group_idx: Option<usize>,
    all_attempts: bool,
    student: Option<String>,
    score_arg: Option<f64>,
    course: Option<String>,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;
    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, course)?;

    // Resolve HW
    let detail = b.course_detail(&course_id).await?;
    let columns = detail.gradebook_columns().await?;
    let hw_cols = filter_hw_columns(&columns);
    let hw_col = resolve_hw(&hw_cols, hw_id)?;
    let possible = hw_col.score.as_ref().map(|s| s.possible).unwrap_or(100.0);

    sp.set_message(format!("fetching grading data for {}...", hw_col.name));
    let data = b
        .load_reconcile_data(&course_id, &hw_col.id)
        .await
        .context("fetch reconcile data")?;

    // Direct grading mode: -s <student> — no group needed
    if let Some(sid) = &student {
        let targets: Vec<&crate::api::blackboard::ReconcileAttempt> = data
            .attempts
            .iter()
            .filter(|a| &a.student_user_id == sid)
            .collect();
        if targets.is_empty() {
            anyhow::bail!("student '{sid}' not found in grading data");
        }
        let name = b.get_user_name(sid).await.unwrap_or_else(|_| sid.clone());
        let existing = targets
            .first()
            .and_then(|a| {
                a._reconciled_score
                    .or_else(|| a.provisional_grades.first().and_then(|pg| pg.score))
            })
            .map(|s| format!(" (当前: {s})"))
            .unwrap_or_default();
        let score_val = match score_arg {
            Some(s) => s,
            None => {
                sp.finish_and_clear();
                let prompt = format!("{name}{existing} (满分 {possible:.0}):");
                let input: String = inquire::Text::new(&prompt).prompt()?;
                input.trim().parse().context("invalid score")?
            }
        };
        let nonce = b
            .get_reconcile_nonce(&course_id, &hw_col.id)
            .await
            .context("get nonce")?;
        sp.set_message("grading...");
        for a in &targets {
            b.save_grade(
                &a.attempt_id,
                &hw_col.id,
                score_val,
                &course_id,
                &nonce,
                None,
            )
            .await?;
        }
        sp.finish_with_message(format!(
            "  {GR}✓{GR:#} {name} = {score_val}  ({D}{} attempts{})",
            targets.len(),
            "{D:#}"
        ));
        return Ok(());
    }

    // Resolve group
    let groups = b.get_course_groups(&course_id).await?;
    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();
    let group = resolve_group(&sub_groups, group_idx)?;

    // Get group members + membership mapping
    let group_members: std::collections::HashSet<String> = b
        .get_group_users(&course_id, &group.id)
        .await?
        .into_iter()
        .collect();
    // Filter
    let mut pending: Vec<&crate::api::blackboard::ReconcileAttempt> = data
        .attempts
        .iter()
        .filter(|a| group_members.contains(&a.student_user_id) && a.status == "NEEDS_GRADING")
        .collect();
    pending.sort_by(|a, b| a.student_user_id.cmp(&b.student_user_id));

    // Deduplicate: keep only latest attempt per student if configured
    if !all_attempts {
        let extract_num = |id: &str| {
            id.strip_prefix('_')
                .and_then(|s| s.rsplit('_').nth(1))
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };
        pending.sort_by(|a, b| {
            let na = extract_num(&a.attempt_id);
            let nb = extract_num(&b.attempt_id);
            nb.cmp(&na)
        });
        let mut seen = std::collections::HashSet::new();
        pending.retain(|a| seen.insert(a.student_user_id.clone()));
        pending.sort_by(|a, b| a.student_user_id.cmp(&b.student_user_id));
    }

    if pending.is_empty() {
        sp.finish_with_message("all submissions are already graded!");
        return Ok(());
    }

    // Get nonce
    sp.set_message("preparing grading session...");
    let nonce = b
        .get_reconcile_nonce(&course_id, &hw_col.id)
        .await
        .context("get nonce")?;

    // Batch grading mode: -S <score> without -s — grade all pending
    if let Some(score_val) = score_arg {
        let total = pending.len();
        sp.finish_and_clear();
        let confirmed = inquire::Confirm::new(&format!(
            "确认给 {B}{}{B:#} 全组 {total} 人打 {RD}{score_val}{RD:#} 分？",
            group.name
        ))
        .with_default(false)
        .prompt()?;
        if !confirmed {
            return Ok(());
        }
        sp.set_message("batch grading...");
        let mut graded = 0usize;
        for a in &pending {
            b.save_grade(
                &a.attempt_id,
                &hw_col.id,
                score_val,
                &course_id,
                &nonce,
                None,
            )
            .await?;
            graded += 1;
            sp.set_message(format!("batch grading [{graded}/{total}]"));
        }
        sp.finish_with_message(format!(
            "  {GR}✓{GR:#} {graded}/{total} graded = {score_val}"
        ));
        return Ok(());
    }

    sp.finish_and_clear();

    let mp = pbar::new_spinner_on(ctx.multi);
    let mut graded = 0usize;
    let total = pending.len();

    for (i, a) in pending.iter().enumerate() {
        let name = b
            .get_user_name(&a.student_user_id)
            .await
            .unwrap_or_else(|_| a.student_user_id.clone());

        let prompt = format!("[{}/{}] {} (满分 {:.0}):", i + 1, total, name, possible);
        let input: String = inquire::Text::new(&prompt)
            .with_help_message("输入分数，q 跳过，e 退出")
            .prompt()?;

        if input.trim().eq_ignore_ascii_case("q") {
            continue;
        }
        if input.trim().eq_ignore_ascii_case("e") {
            break;
        }

        let score: f64 = match input.trim().parse() {
            Ok(s) => s,
            Err(_) => {
                println!("  {RD}无效分数，跳过{RD:#}");
                continue;
            }
        };

        let feedback_prompt = format!("[{}/{}] {} 评语（可选，回车跳过）:", i + 1, total, name,);
        let feedback: String = inquire::Text::new(&feedback_prompt)
            .with_help_message("输入评语，回车跳过")
            .prompt()?;
        let feedback_opt = if feedback.trim().is_empty() {
            None
        } else {
            Some(feedback.trim().to_owned())
        };

        mp.set_message(format!(
            "[{}/{}] saving grade {}={}...",
            i + 1,
            total,
            name,
            score
        ));

        match b
            .save_grade(
                &a.attempt_id,
                &hw_col.id,
                score,
                &course_id,
                &nonce,
                feedback_opt.as_deref(),
            )
            .await
        {
            Ok(_) => {
                graded += 1;
                let fb_info = feedback_opt
                    .as_ref()
                    .map(|f| format!(" [评语: {}]", f))
                    .unwrap_or_default();
                println!("  {GR}✓ 已保存: {}={}{fb_info}{GR:#}", name, score);

                // Auto-grade earlier attempts with same score
                if !all_attempts {
                    let extract_num = |id: &str| {
                        id.strip_prefix('_')
                            .and_then(|s| s.rsplit('_').nth(1))
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(0)
                    };
                    let latest_num = extract_num(&a.attempt_id);
                    for other in &data.attempts {
                        if other.student_user_id == a.student_user_id
                            && other.attempt_id != a.attempt_id
                            && extract_num(&other.attempt_id) < latest_num
                            && other.status == "NEEDS_GRADING"
                        {
                            log::info!(
                                "auto-grading earlier attempt {} for {} with score {}",
                                other.attempt_id,
                                name,
                                score
                            );
                            if let Err(e) = b
                                .save_grade(
                                    &other.attempt_id,
                                    &hw_col.id,
                                    score,
                                    &course_id,
                                    &nonce,
                                    None,
                                )
                                .await
                            {
                                log::warn!(
                                    "failed to auto-grade attempt {}: {e:#}",
                                    other.attempt_id
                                );
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!("  {RD}✗ 保存失败: {e:#}{RD:#}");
            }
        }
    }

    mp.finish_and_clear();

    writeln!(
        std::io::stdout(),
        "{D}>{D:#} {GR}完成: {graded}/{total} 个评分已提交{GR:#}{D}<{D:#}"
    )?;

    Ok(())
}

// ── Helpers ──

fn resolve_group<'a>(
    groups: &[&'a CourseGroup],
    idx: Option<usize>,
) -> anyhow::Result<&'a CourseGroup> {
    if groups.is_empty() {
        anyhow::bail!("no groups found in this course");
    }
    if let Some(gid) = idx {
        groups
            .get(gid.wrapping_sub(1))
            .copied()
            .context("invalid group index")
    } else {
        select_group_interactive(groups)
    }
}

fn resolve_hw<'a>(
    hw_cols: &[&'a crate::api::blackboard::GradebookColumn],
    id: Option<usize>,
) -> anyhow::Result<&'a crate::api::blackboard::GradebookColumn> {
    if hw_cols.is_empty() {
        anyhow::bail!("no assignment columns found");
    }
    if let Some(hid) = id {
        hw_cols
            .get(hid.wrapping_sub(1))
            .copied()
            .context("invalid assignment index")
    } else {
        let items: Vec<String> = hw_cols.iter().map(|c| c.name.clone()).collect();
        let selection = inquire::Select::new("选择作业:", items).prompt()?;
        let idx = hw_cols.iter().position(|c| c.name == selection).unwrap();
        Ok(hw_cols[idx])
    }
}

fn select_course(
    enrollments: &[crate::api::blackboard::CourseEnrollment],
    preferred: Option<String>,
) -> anyhow::Result<String> {
    if enrollments.is_empty() {
        anyhow::bail!("no TeachingAssistant courses found (hint: check your account permissions)");
    }
    if enrollments.len() == 1 {
        return Ok(enrollments[0].course_id.clone());
    }
    if let Some(ref cid) = preferred {
        if enrollments.iter().any(|e| e.course_id == *cid) {
            return Ok(cid.clone());
        }
        anyhow::bail!("course '{cid}' not found in your TA enrollments");
    }
    let items: Vec<String> = enrollments
        .iter()
        .map(|e| {
            let name = e.course.as_ref().map(|c| c.name.as_str()).unwrap_or("?");
            let term = e
                .course
                .as_ref()
                .and_then(|c| c.term.as_ref())
                .map(|t| t.name.as_str())
                .unwrap_or("");
            format!("{name}  {term}  ({})", e.course_id)
        })
        .collect();
    let selection = inquire::Select::new("选择课程:", items.clone()).prompt()?;
    // Extract course_id from the selected item
    let idx = items.iter().position(|i| *i == selection).unwrap();
    Ok(enrollments[idx].course_id.clone())
}

fn filter_hw_columns(
    columns: &[crate::api::blackboard::GradebookColumn],
) -> Vec<&crate::api::blackboard::GradebookColumn> {
    columns
        .iter()
        .filter(|c| {
            c.grading
                .as_ref()
                .is_some_and(|g| g.grading_type == "Attempts")
        })
        .collect()
}

fn select_group_interactive<'a>(groups: &[&'a CourseGroup]) -> anyhow::Result<&'a CourseGroup> {
    let items: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
    let selection = inquire::Select::new("选择批改组:", items).prompt()?;
    let idx = groups.iter().position(|g| g.name == selection).unwrap();
    Ok(groups[idx])
}
