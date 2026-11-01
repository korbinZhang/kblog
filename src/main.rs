mod admin;
mod config;
mod db;
mod markdown;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::{Router, response::Html, routing::get};
use chrono::{Datelike, Local};
use config::Config;
use db::Database;
use minijinja::{AutoEscape, Environment, Value, context};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing;
use tracing_subscriber;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    env: Environment<'static>,
    db: Database,
    cfg: Config,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    tracing::info!("日志初始化成功!");

    let config = Config::init();

    let db = Database::init(&config).await;
    admin::ensure_admin_password(&db).await;

    let mut env = Environment::new();
    // 模板后缀是 .jinja, MiniJinja 默认的自动转义回调不覆盖它, 必须显式打开,
    // 否则后台写入的标题/描述会直接成为持久化 XSS。
    env.set_auto_escape_callback(|_| AutoEscape::Html);
    env.add_function("startswith", |s: &str, prefix: &str| s.starts_with(prefix));
    env.set_loader(minijinja::path_loader(config.templates_path.clone()));

    let app_state = AppState { env, db, cfg: config.clone() };

    let app = Router::new()
        .route("/", get(handler_archives_last))
        .route("/blog", get(handler_archives_last))
        .route("/blog/{year}", get(handler_archives))
        .route("/blog/{year}/{slug}", get(handler_post))
        .route("/thinking", get(handler_thinking_last))
        .route("/thinking/{year}", get(handler_thinking))
        .route("/about", get(handler_about))
        .route(
            &format!("{}/", config.admin_prefix()),
            get(admin::handler_trailing_slash),
        )
        .nest(&config.admin_prefix(), admin::router(&app_state))
        .fallback_service(ServeDir::new(config.assets_path.clone()))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.web_addr, config.web_port))
            .await
            .unwrap();
    tracing::info!("服务器启动: http://{}:{}", config.web_addr, config.web_port);
    tracing::info!(
        "后台登录页: http://{}:{}{}/login",
        config.web_addr,
        config.web_port,
        config.admin_prefix()
    );
    let _ = axum::serve(listener, app).await;
}

fn current_year() -> u16 {
    Local::now().naive_local().year() as u16
}

async fn ctx_site(db: &Database, path: &str, title: Option<&str>, description: Option<&str>) -> Value {
    let settings = db.all_settings().await.unwrap_or_default();
    let get = |key: &str| settings.get(key).cloned().unwrap_or_default();
    let site_name = get("site_name");
    let navs = vec![("/blog", "文章"), ("/thinking", "碎念"), ("/about", "关于")];
    let navs: Vec<_> = navs
        .iter()
        .map(|x| (x.0, x.1, path.starts_with(x.0)))
        .collect();
    context! {
        title => title.map(str::to_string).unwrap_or(site_name.clone()),
        description => description.map(str::to_string).unwrap_or_else(|| get("site_description")),
        brand => site_name,
        site_url => get("site_url"),
        navs => navs,
    }
}

async fn handler_archives_last(
    State(state): State<AppState>,
    req: Request,
) -> Result<Html<String>, StatusCode> {
    let year = state.db.latest_post_year().await.unwrap_or_else(current_year);
    render_archives(state, req, year).await
}

async fn handler_archives(
    State(state): State<AppState>,
    Path(year): Path<u16>,
    req: Request,
) -> Result<Html<String>, StatusCode> {
    render_archives(state, req, year).await
}

async fn handler_post(
    State(state): State<AppState>,
    Path((year, slug)): Path<(u16, String)>,
    req: Request,
) -> Result<Html<String>, StatusCode> {
    let _ = year;
    let post = state
        .db
        .post_by_slug(&slug)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let (dt, title, _slug, description, content) = post;

    let template = state.env.get_template("post.jinja").unwrap();
    let rendered = template
        .render(context! {
            post_title => &title,
            post_iso_date => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            post_date => dt.format("%Y-%m-%d").to_string(),
            post_content => markdown::to_html(&content),
            ..ctx_site(&state.db, req.uri().path(), Some(title.as_str()), Some(description.as_str())).await,
        })
        .unwrap();
    Ok(Html(rendered))
}

async fn render_archives(
    state: AppState,
    req: Request,
    year: u16,
) -> Result<Html<String>, StatusCode> {
    let year_counts = state.db.post_year_counts().await;
    let toolbar_tabs: Vec<_> = year_counts
        .iter()
        .map(|&(y, count)| (y, count, y == year))
        .collect();
    let current_count = year_counts
        .iter()
        .find(|&&(y, _)| y == year)
        .map(|&(_, c)| c)
        .unwrap_or(0);
    let total_count: i64 = year_counts.iter().map(|&(_, c)| c).sum();

    let rows = state.db.posts_by_year(year).await.unwrap_or_default();
    let posts: Vec<_> = rows
        .iter()
        .map(|(dt, title, slug, _description, content)| {
            let word_count = strip_html_tags(&markdown::to_html(content))
                .chars()
                .count();
            context! {
                iso_date => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                date => dt.format("%Y-%m-%d").to_string(),
                year => year,
                title => title,
                slug => slug,
                word_count => word_count,
            }
        })
        .collect();

    let template = state.env.get_template("blog.jinja").unwrap();
    let rendered = template
        .render(context! {
            base_url => "/blog",
            total_count => total_count,
            toolbar_tabs,
            current_year => year,
            current_count,
            posts,
            ..ctx_site(&state.db, req.uri().path(), None, None).await,
        })
        .unwrap();
    Ok(Html(rendered))
}

fn strip_html_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

async fn render_thinking(
    state: AppState,
    req: Request,
    year: u16,
) -> Result<Html<String>, StatusCode> {
    let year_counts = state.db.year_counts().await;
    let toolbar_tabs: Vec<_> = year_counts
        .iter()
        .map(|&(y, count)| (y, count, y == year))
        .collect();
    let current_count = year_counts
        .iter()
        .find(|&&(y, _)| y == year)
        .map(|&(_, c)| c)
        .unwrap_or(0);
    let total_count: i64 = year_counts.iter().map(|&(_, c)| c).sum();

    let rows = state.db.thinking_by_year(year).await.unwrap_or_default();
    let total_for_year = rows.len();
    let contents: Vec<(String, String, String, String)> = rows
        .iter()
        .enumerate()
        .map(|(i, (dt, content))| {
            (
                dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                dt.format("%Y年%m月%d日 %H时%M分").to_string(),
                (total_for_year - i).to_string(),
                content.clone(),
            )
        })
        .collect();

    let template = state.env.get_template("thinkings.jinja").unwrap();
    let rendered = template
        .render(context! {
            base_url => "/thinking",
            total_count => total_count,
            toolbar_tabs,
            current_year => year,
            current_count,
            contents,
            ..ctx_site(&state.db, req.uri().path(), None, None).await,
        })
        .unwrap();
    Ok(Html(rendered))
}

async fn handler_thinking_last(
    State(state): State<AppState>,
    req: Request,
) -> Result<Html<String>, StatusCode> {
    let year = state.db.latest_year().await.unwrap_or_else(current_year);
    render_thinking(state, req, year).await
}

async fn handler_thinking(
    State(state): State<AppState>,
    Path(year): Path<u16>,
    req: Request,
) -> Result<Html<String>, StatusCode> {
    render_thinking(state, req, year).await
}

async fn handler_about(
    State(state): State<AppState>,
    req: Request,
) -> Result<Html<String>, StatusCode> {
    let about_content = state
        .db
        .get_setting("about_content")
        .await
        .unwrap_or_default();

    let template = state.env.get_template("about.jinja").unwrap();
    let rendered = template
        .render(context! {
            about_content => about_content,
            ..ctx_site(&state.db, req.uri().path(), Some("关于"), Some("关于我")).await,
        })
        .unwrap();
    Ok(Html(rendered))
}
