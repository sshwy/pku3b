use super::*;

use crate::api::blackboard::{AttemptFileInfo, CourseGroup, CourseMembership};
use crate::config;
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
    },
    /// 查看评分复核状态
    Review {
        /// 作业编号（不填则交互选择）
        id: Option<usize>,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        #[arg(short, long)]
        group: Option<usize>,
    },
    /// 登分（给作业打分）
    Grade {
        /// 作业编号（不填则交互选择）
        id: Option<usize>,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        #[arg(short, long)]
        group: Option<usize>,
        /// 复查已评分提交
        #[arg(short, long, default_value = "false")]
        recheck: bool,
        /// 下载/评分全部历史提交（默认只取最新一次）
        #[arg(short = 'A', long, default_value = "false")]
        all_attempts: bool,
    },
    /// 下载作业提交文件
    Down {
        /// 作业编号（不填则交互选择）
        id: Option<usize>,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "false")]
        all_term: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
        /// 批改组编号
        #[arg(short, long)]
        group: Option<usize>,
        /// 只下载未评分的
        #[arg(short = 'u', long, default_value = "false")]
        ungraded: bool,
        /// 下载全部（含已评分）
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
    },
    /// 查看组成员
    Show {
        id: usize,
        #[arg(short, long, default_value = "false")]
        force: bool,
        #[arg(long, default_value = "")]
        otp_code: String,
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
            } => ta_hw_ls(ctx, force, all_term, otp_code, group).await?,
            TaHwCommands::Review {
                id,
                force,
                otp_code,
                group,
            } => ta_review(ctx, id, force, otp_code, group).await?,
            TaHwCommands::Grade {
                id,
                force,
                otp_code,
                group,
                recheck,
                all_attempts,
            } => ta_grade(ctx, id, force, otp_code, group, recheck, all_attempts).await?,
            TaHwCommands::Down {
                id,
                force,
                all_term,
                otp_code,
                group,
                ungraded,
                all,
                all_hw,
                no_rename,
                all_attempts,
            } => {
                ta_hw_down(
                    ctx,
                    id,
                    force,
                    all_term,
                    otp_code,
                    group,
                    ungraded,
                    all,
                    all_hw,
                    no_rename,
                    all_attempts,
                )
                .await?
            }
        },
        TaCommands::Group { command } => match command {
            TaGroupCommands::Ls { force, otp_code } => ta_group_ls(ctx, force, otp_code).await?,
            TaGroupCommands::Show {
                id,
                force,
                otp_code,
            } => ta_group_show(ctx, id, force, otp_code).await?,
        },
    }
    Ok(())
}

// ── Group commands ──

async fn ta_group_ls(ctx: &CommandCtx<'_>, force: bool, otp_code: String) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config")?;
    let course_id = select_course(&ta_courses, &cfg)?;

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
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config")?;
    let course_id = select_course(&ta_courses, &cfg)?;

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
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config")?;
    let course_id = select_course(&ta_courses, &cfg)?;

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
    ungraded: bool,
    all: bool,
    all_hw: bool,
    no_rename: bool,
    all_attempts: bool,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;

    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config")?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, &cfg)?;

    // Resolve group
    sp.set_message("fetching groups...");
    let groups = b.get_course_groups(&course_id).await?;
    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();

    if sub_groups.is_empty() {
        anyhow::bail!("no grading groups found in this course");
    }

    let group = if let Some(gid) = group_idx {
        sub_groups
            .get(gid.wrapping_sub(1))
            .map(|g| *g)
            .context("invalid group index")?
    } else if let Some(ref default_gid) = cfg.ta_group_id {
        match sub_groups.iter().find(|g| g.id == *default_gid) {
            Some(g) => g,
            None => {
                log::warn!("configured ta_group_id not found, selecting interactively...");
                select_group_interactive(&sub_groups)?
            }
        }
    } else {
        select_group_interactive(&sub_groups)?
    };

    sp.set_message(format!("fetching members of {}...", group.name));
    let group_members: std::collections::HashSet<String> = b
        .get_group_users(&course_id, &group.id)
        .await?
        .into_iter()
        .collect();

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
        hw_cols.iter().copied().collect()
    } else {
        vec![resolve_hw(&hw_cols, hw_id)?]
    };

    let dir = std::path::PathBuf::from(".");
    if !dir.exists() {
        fs::create_dir_all(&dir).await?;
    }

    let mp = pbar::new_spinner_on(ctx.multi);
    sp.finish_and_clear();

    for hw_col in target_hw {
        log::info!("fetching submissions for {}...", hw_col.name);
        let mut attempts = detail.get_attempts(&hw_col.id).await?;

        // Filter: group members only
        attempts.retain(|a| group_members.contains(&a.user_id));

        // Default to ungraded only when all_hw
        let only_ungraded = if all_hw { true } else { ungraded && !all };
        if only_ungraded {
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
            group.name,
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

// ── Review & Grade commands ──

async fn ta_review(
    ctx: &CommandCtx<'_>,
    hw_id: Option<usize>,
    force: bool,
    otp_code: String,
    group_idx: Option<usize>,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;
    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config")?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, &cfg)?;

    // Resolve group
    sp.set_message("fetching groups...");
    let groups = b.get_course_groups(&course_id).await?;
    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();
    let group = resolve_group(&sub_groups, group_idx, &cfg)?;

    // Resolve HW
    let detail = b.course_detail(&course_id).await?;
    let columns = detail.gradebook_columns().await?;
    let hw_cols = filter_hw_columns(&columns);
    let hw_col = resolve_hw(&hw_cols, hw_id)?;

    sp.set_message(format!("fetching review data for {}...", hw_col.name));

    // Get reconcile data
    let data = b
        .load_reconcile_data(&course_id, &hw_col.id)
        .await
        .context("fetch reconcile data")?;

    // Get group members and names
    let group_members: std::collections::HashSet<String> = b
        .get_group_users(&course_id, &group.id)
        .await?
        .into_iter()
        .collect();

    // Filter attempts by group members
    let mut group_attempts: Vec<&crate::api::blackboard::ReconcileAttempt> = data
        .attempts
        .iter()
        .filter(|a| group_members.contains(&a.student_user_id))
        .collect();
    group_attempts.sort_by(|a, b| a.student_user_id.cmp(&b.student_user_id));

    let graded = group_attempts
        .iter()
        .filter(|a| a.status == "COMPLETED")
        .count();
    let needs_review = group_attempts
        .iter()
        .filter(|a| a.status == "NEEDS_GRADING")
        .count();

    let my_id = &user_id;

    sp.finish_with_message("done.");

    let mut outbuf = Vec::new();
    writeln!(
        outbuf,
        "{D}>{D:#} {B}{} — {}{B:#} {D}<{D:#}\n",
        hw_col.name, group.name
    )?;
    writeln!(
        outbuf,
        "  {D}{:>4} {:<25} {:<12} {:<8} {}{D:#}",
        "#", "学生", "状态", "分数", "批改人"
    )?;

    // Collect user names
    let mut names = std::collections::HashMap::new();
    for a in &group_attempts {
        if !names.contains_key(&a.student_user_id) {
            let name = b
                .get_user_name(&a.student_user_id)
                .await
                .unwrap_or_default();
            names.insert(a.student_user_id.clone(), name);
        }
    }

    for (i, a) in group_attempts.iter().enumerate() {
        let name = names
            .get(&a.student_user_id)
            .map(|s| s.as_str())
            .unwrap_or("?");
        let grade_info = a.provisional_grades.first();
        let grader = grade_info
            .map(|pg| {
                if pg.grader_user_id == *my_id {
                    format!("{GR}{} (你){GR:#}", pg.grader_user_id)
                } else {
                    pg.grader_user_id.clone()
                }
            })
            .unwrap_or_else(|| "-".to_string());
        let score_str = a
            .reconciled_score
            .map(|s| format!("{:.1}", s))
            .unwrap_or_else(|| "-".to_string());

        write!(outbuf, "  {D}{:>4}{D:#}  ", i + 1)?;
        write!(outbuf, "{:<25}  ", name)?;
        if a.status == "COMPLETED" {
            write!(outbuf, "{GR}COMPLETED   {GR:#}")?;
        } else {
            write!(outbuf, "{RD}NEEDS_GRADING{RD:#}")?;
        }
        writeln!(outbuf, "  {:<8}  {}", score_str, grader)?;
    }

    writeln!(
        outbuf,
        "\n{D}已批改: {GR}{graded}{D:#}/{GR}{}{D:#}    需要批改: {RD}{needs_review}{D:#}",
        group_attempts.len()
    )?;

    buf_try!(@try fs::stdout().write_all(outbuf).await);
    Ok(())
}

async fn ta_grade(
    ctx: &CommandCtx<'_>,
    hw_id: Option<usize>,
    force: bool,
    otp_code: String,
    group_idx: Option<usize>,
    recheck: bool,
    all_attempts: bool,
) -> anyhow::Result<()> {
    let (b, sp) = load_blackboard(ctx, otp_code, force).await?;
    let cfg = config::read_cfg(&ctx.config_path)
        .await
        .context("read config")?;

    sp.set_message("fetching courses...");
    let user_id = b.user_info_id().await?;
    let ta_courses = b.get_ta_courses(&user_id).await?;
    let course_id = select_course(&ta_courses, &cfg)?;

    // Resolve group
    let groups = b.get_course_groups(&course_id).await?;
    let sub_groups: Vec<&CourseGroup> = groups.iter().filter(|g| !g.is_group_set).collect();
    let group = resolve_group(&sub_groups, group_idx, &cfg)?;

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
        .filter(|a| {
            let in_group = group_members.contains(&a.student_user_id);
            if recheck {
                in_group
            } else {
                in_group && a.status == "NEEDS_GRADING"
            }
        })
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

    sp.finish_and_clear();

    let mp = pbar::new_spinner_on(ctx.multi);
    let mut graded = 0usize;
    let total = pending.len();

    for (i, a) in pending.iter().enumerate() {
        let name = b
            .get_user_name(&a.student_user_id)
            .await
            .unwrap_or_else(|_| a.student_user_id.clone());

        // If recheck, show existing grade
        if recheck {
            let existing = a.provisional_grades.first();
            if let Some(pg) = existing {
                println!(
                    "  [{}/{}] {} 当前分数: {:.1} (状态: {})",
                    i + 1,
                    total,
                    name,
                    pg.score.unwrap_or(0.0),
                    pg.status
                );
            }
        }

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
    cfg: &config::Config,
) -> anyhow::Result<&'a CourseGroup> {
    if groups.is_empty() {
        anyhow::bail!("no groups found in this course");
    }
    if let Some(gid) = idx {
        groups
            .get(gid.wrapping_sub(1))
            .copied()
            .context("invalid group index")
    } else if let Some(ref default_gid) = cfg.ta_group_id {
        groups
            .iter()
            .find(|g| g.id == *default_gid)
            .copied()
            .context("configured ta_group_id not found")
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
    cfg: &config::Config,
) -> anyhow::Result<String> {
    if enrollments.is_empty() {
        anyhow::bail!("no TeachingAssistant courses found (hint: check your account permissions)");
    }
    if enrollments.len() == 1 {
        return Ok(enrollments[0].course_id.clone());
    }
    if let Some(ref cid) = cfg.ta_course_id {
        if enrollments.iter().any(|e| e.course_id == *cid) {
            return Ok(cid.clone());
        }
    }
    let items: Vec<String> = enrollments.iter().map(|e| e.course_id.clone()).collect();
    let selection = inquire::Select::new("选择课程:", items).prompt()?;
    Ok(selection)
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
