use anyhow::Result;
use chrono::{Local, TimeZone};
use clap::{Parser, Subcommand};
use remem::{context, db, install, mcp, observe, summarize};

#[derive(Parser)]
#[command(name = "remem", about = "Persistent memory for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate context for SessionStart hook (stdout → CLAUDE.md)
    Context {
        /// Working directory (defaults to CWD)
        #[arg(long)]
        cwd: Option<String>,
        /// Session ID
        #[arg(long)]
        session_id: Option<String>,
        /// Use color output
        #[arg(long)]
        color: bool,
    },
    /// Initialize/update session from UserPromptSubmit hook (stdin JSON)
    SessionInit,
    /// Extract observations from PostToolUse hook (stdin JSON)
    Observe,
    /// Stop hook dispatcher: spawn background worker, return immediately
    Summarize,
    /// Background worker: actual summarization (called by Summarize, not by hooks)
    SummarizeWorker,
    /// Flush pending observation queue (batch process with one AI call)
    Flush {
        /// Session ID
        #[arg(long)]
        session_id: String,
        /// Project name
        #[arg(long)]
        project: String,
    },
    /// Run MCP server (stdio transport, long-running)
    Mcp,
    /// Install hooks + MCP to ~/.claude/settings.json
    Install,
    /// Uninstall hooks + MCP from ~/.claude/settings.json
    Uninstall,
    /// 清理旧数据：删除孤立 summary、重复 summary、过期 pending
    Cleanup,
    /// 统计 AI token 消耗与成本（单次 + 按天）
    Usage {
        /// 统计最近 N 天（默认 7）
        #[arg(long, default_value_t = 7)]
        days: i64,
        /// 仅统计今天（本地时区自然日）
        #[arg(long)]
        today: bool,
        /// 显示最近 N 次调用明细（默认 20）
        #[arg(long, default_value_t = 20)]
        limit: i64,
        /// 仅统计指定项目
        #[arg(long)]
        project: Option<String>,
        /// 导出 CSV（包含 totals/daily/events 三类记录）
        #[arg(long)]
        csv: Option<String>,
    },
}

fn local_day_start_epoch() -> i64 {
    let now = Local::now();
    let date = now.date_naive();
    date.and_hms_opt(0, 0, 0)
        .and_then(|naive| Local.from_local_datetime(&naive).single())
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| now.timestamp() - 86400)
}

fn csv_escape(raw: &str) -> String {
    if raw.contains(',') || raw.contains('"') || raw.contains('\n') || raw.contains('\r') {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

fn write_usage_csv(
    path: &str,
    totals: &db::AiUsageTotals,
    daily: &[db::DailyAiUsage],
    events: &[db::AiUsageEvent],
) -> Result<()> {
    let path_buf = std::path::PathBuf::from(path);
    if let Some(parent) = path_buf.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut out = String::new();
    out.push_str("section,day,created_at,project,operation,executor,model,calls,input_tokens,output_tokens,total_tokens,estimated_cost_usd\n");
    let mut push_row = |fields: Vec<String>| {
        out.push_str(&fields.join(","));
        out.push('\n');
    };
    push_row(vec![
        "totals".to_string(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        totals.calls.to_string(),
        totals.input_tokens.to_string(),
        totals.output_tokens.to_string(),
        totals.total_tokens.to_string(),
        format!("{:.6}", totals.estimated_cost_usd),
    ]);
    for d in daily {
        push_row(vec![
            "daily".to_string(),
            csv_escape(&d.day),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            d.calls.to_string(),
            d.input_tokens.to_string(),
            d.output_tokens.to_string(),
            d.total_tokens.to_string(),
            format!("{:.6}", d.estimated_cost_usd),
        ]);
    }
    for e in events {
        push_row(vec![
            "event".to_string(),
            String::new(),
            csv_escape(&e.created_at),
            csv_escape(e.project.as_deref().unwrap_or("-")),
            csv_escape(&e.operation),
            csv_escape(&e.executor),
            csv_escape(e.model.as_deref().unwrap_or("-")),
            String::new(),
            e.input_tokens.to_string(),
            e.output_tokens.to_string(),
            e.total_tokens.to_string(),
            format!("{:.6}", e.estimated_cost_usd),
        ]);
    }
    std::fs::write(path_buf, out)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Context {
            cwd,
            session_id,
            color,
        } => {
            let cwd = cwd.unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            context::generate_context(&cwd, session_id.as_deref(), color)?;
        }
        Commands::SessionInit => {
            observe::session_init().await?;
        }
        Commands::Observe => {
            observe::observe().await?;
        }
        Commands::Summarize => {
            summarize::summarize().await?;
        }
        Commands::SummarizeWorker => {
            summarize::summarize_worker().await?;
        }
        Commands::Flush {
            session_id,
            project,
        } => {
            observe::flush_pending(&session_id, &project).await?;
        }
        Commands::Mcp => {
            mcp::run_mcp_server().await?;
        }
        Commands::Install => {
            install::install()?;
        }
        Commands::Uninstall => {
            install::uninstall()?;
        }
        Commands::Cleanup => {
            let conn = db::open_db()?;
            let orphans = db::cleanup_orphan_summaries(&conn)?;
            let dupes = db::cleanup_duplicate_summaries(&conn)?;
            let stale = db::cleanup_stale_pending(&conn)?;
            let expired = db::cleanup_expired_compressed(&conn, 90)?;
            println!("清理完成:");
            println!("  孤立 summary (无对应 observation): {} 条", orphans);
            println!("  重复 summary (同 session 多条): {} 条", dupes);
            println!("  过期 pending (超 1 小时): {} 条", stale);
            println!("  过期 compressed (超 90 天): {} 条", expired);
        }
        Commands::Usage {
            days,
            today,
            limit,
            project,
            csv,
        } => {
            let days = days.max(1);
            let limit = limit.max(1);
            let conn = db::open_db()?;
            let (totals, daily, events) = if today {
                let from_epoch = local_day_start_epoch();
                (
                    db::query_ai_usage_totals_since(&conn, from_epoch, project.as_deref())?,
                    db::query_ai_usage_daily_since(&conn, from_epoch, project.as_deref())?,
                    db::query_ai_usage_events_since(&conn, from_epoch, limit, project.as_deref())?,
                )
            } else {
                (
                    db::query_ai_usage_totals(&conn, days, project.as_deref())?,
                    db::query_ai_usage_daily(&conn, days, project.as_deref())?,
                    db::query_ai_usage_events(&conn, days, limit, project.as_deref())?,
                )
            };

            let scope = project
                .as_deref()
                .map(|p| format!(" / 项目 {}", p))
                .unwrap_or_default();
            if today {
                let day = Local::now().format("%Y-%m-%d").to_string();
                println!("AI 用量统计（今天 {}{}）", day, scope);
            } else {
                println!("AI 用量统计（最近 {} 天{}）", days, scope);
            }
            println!("  调用次数: {}", totals.calls);
            println!("  Input tokens: {}", totals.input_tokens);
            println!("  Output tokens: {}", totals.output_tokens);
            println!("  Total tokens: {}", totals.total_tokens);
            println!("  额外支出(估算): ${:.6}", totals.estimated_cost_usd);

            println!("\n按天汇总:");
            if daily.is_empty() {
                println!("  (无数据)");
            } else {
                for d in &daily {
                    println!(
                        "  {} | calls={} in={} out={} total={} cost=${:.6}",
                        d.day,
                        d.calls,
                        d.input_tokens,
                        d.output_tokens,
                        d.total_tokens,
                        d.estimated_cost_usd
                    );
                }
            }

            println!("\n最近每次调用:");
            if events.is_empty() {
                println!("  (无数据)");
            } else {
                for e in &events {
                    println!(
                        "  {} | project={} op={} exec={} model={} in={} out={} total={} cost=${:.6}",
                        e.created_at,
                        e.project.as_deref().unwrap_or("-"),
                        e.operation,
                        e.executor,
                        e.model.as_deref().unwrap_or("-"),
                        e.input_tokens,
                        e.output_tokens,
                        e.total_tokens,
                        e.estimated_cost_usd
                    );
                }
            }

            if let Some(csv_path) = csv.as_deref() {
                write_usage_csv(csv_path, &totals, &daily, &events)?;
                println!("\nCSV 已导出: {}", csv_path);
            }

            println!("\n注: 成本按 REMEM_PRICE_* 环境变量或内置默认单价估算，单位为 USD。");
        }
    }

    Ok(())
}
