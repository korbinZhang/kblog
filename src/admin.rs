use super::db::{Database, NewPost, Post, random_password, random_token};
use super::AppState;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::extract::{Form, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use chrono::{Local, NaiveDateTime};
use minijinja::{Value, context};
use serde::Deserialize;

const SESSION_COOKIE: &str = "kblog_session";
const SESSION_TTL_DAYS: i64 = 7;
const PER_PAGE: i64 = 20;
const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 500;
const MAX_CONTENT: usize = 200_000;

pub async fn ensure_admin_password(db: &Database) {
    if db.get_setting("admin_password").await.is_some() {
        return;
    }
    let password = random_password();
    match hash_password(&password) {
        Ok(hash) => match db.set_settings(&[("admin_password", &hash)]).await {
            Ok(()) => tracing::info!(
                "已生成后台初始密码: {password}  (登录地址见下方启动日志, 请登录后立即修改)"
            ),
            Err(err) => tracing::error!("后台初始密码写入失败: {err}"),
        },
        Err(err) => tracing::error!("后台初始密码哈希失败: {err}"),
    }
}

pub fn router(state: &AppState) -> axum::Router<AppState> {
    use axum::routing::{get, post};
    use axum::Router;

    let protected = Router::new()
        .route("/", get(handler_dashboard))
        .route("/settings", get(handler_settings).post(handler_settings_save))
        .route("/password", post(handler_password_change))
        .route("/posts", get(handler_posts))
        .route("/posts/new", get(handler_post_new).post(handler_post_create))
        .route("/posts/{id}", get(handler_post_edit).post(handler_post_update))
        .route("/posts/{id}/publish", post(handler_post_publish))
        .route("/posts/{id}/unpublish", post(handler_post_unpublish))
        .route("/posts/{id}/delete", post(handler_post_delete))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_admin,
        ));

    Router::new()
        .route("/login", get(handler_login).post(handler_login_post))
        .route("/logout", post(handler_logout))
        .merge(protected)
}

// ---------------------------------------------------------------- 鉴权

pub async fn require_admin(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let valid = match jar.get(SESSION_COOKIE) {
        Some(cookie) => state.db.session_is_valid(cookie.value()).await.unwrap_or(false),
        None => false,
    };
    if valid {
        next.run(req).await
    } else {
        // 与「路径根本不存在」不可区分, 不暴露后台形状
        StatusCode::NOT_FOUND.into_response()
    }
}

#[derive(Deserialize)]
pub struct LoginForm {
    password: String,
}

pub async fn handler_login(State(state): State<AppState>, jar: CookieJar) -> Response {
    if is_logged_in(&state, &jar).await {
        return dashboard_url(&state).into_response();
    }
    render(&state, "admin/login.jinja", ctx_admin(&state, "login", None, false).await)
}

pub async fn handler_login_post(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Response {
    let stored = state.db.get_setting("admin_password").await.unwrap_or_default();
    if !verify_password(&form.password, &stored) {
        return render(&state, "admin/login.jinja", ctx_admin(&state, "login", Some("密码不正确"), false).await);
    }

    let token = random_token();
    if state.db.create_session(&token, SESSION_TTL_DAYS).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let jar = jar.add(session_cookie(&state, &token));
    (jar, dashboard_url(&state)).into_response()
}

pub async fn handler_logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        let _ = state.db.delete_session(cookie.value()).await;
    }
    let jar = jar.remove(session_cookie(&state, ""));
    (jar, Redirect::to(&login_url(&state))).into_response()
}

async fn is_logged_in(state: &AppState, jar: &CookieJar) -> bool {
    match jar.get(SESSION_COOKIE) {
        Some(cookie) => state.db.session_is_valid(cookie.value()).await.unwrap_or(false),
        None => false,
    }
}

fn session_cookie(state: &AppState, token: &str) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token.to_string()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path(state.cfg.admin_prefix())
        .max_age(time::Duration::days(SESSION_TTL_DAYS))
        .build()
}

fn dashboard_url(state: &AppState) -> Response {
    Redirect::to(&format!("{}/", state.cfg.admin_prefix())).into_response()
}

// axum 的 nest 只把不带斜杠的 /{admin} 交给子路由的 "/" handler, 而登录后要落在带斜杠的地址。
// 未登录时该路径必须和任意未知路径一样返回 404, 所以只能在挂载点自己判定会话。
pub async fn handler_trailing_slash(State(state): State<AppState>, jar: CookieJar) -> Response {
    if is_logged_in(&state, &jar).await {
        Redirect::to(&state.cfg.admin_prefix()).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

fn login_url(state: &AppState) -> String {
    format!("{}/login", state.cfg.admin_prefix())
}

fn admin_url(state: &AppState, tail: &str) -> String {
    format!("{}{}", state.cfg.admin_prefix(), tail)
}

fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    Ok(Argon2::default().hash_password(password.as_bytes())?.to_string())
}

fn verify_password(input: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|parsed| Argon2::default().verify_password(input.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

// ---------------------------------------------------------------- 公共上下文

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Flash {
    saved: Option<String>,
    err: Option<String>,
    page: Option<i64>,
}

fn flash_message(flash: &Flash) -> Option<&'static str> {
    match (flash.saved.as_deref(), flash.err.as_deref()) {
        (Some(_), _) => Some("已保存"),
        (None, Some("slug")) => Some("该 slug 已被其他文章占用"),
        (None, Some("empty")) => Some("标题不能为空"),
        (None, Some("length")) => Some("内容超出长度限制"),
        (None, Some("datetime")) => Some("文章日期格式不正确"),
        (None, Some("password")) => Some("当前密码不正确"),
        (None, Some("mismatch")) => Some("两次输入的新密码不一致"),
        (None, Some("db")) => Some("数据库写入失败, 请查看服务端日志"),
        (None, Some(_)) => Some("操作未完成"),
        (None, None) => None,
    }
}

async fn ctx_admin(
    state: &AppState,
    page_title: &str,
    message: Option<&str>,
    logged_in: bool,
) -> Value {
    let settings = state.db.all_settings().await.unwrap_or_default();
    context! {
        admin_base => state.cfg.admin_prefix(),
        site_name => settings.get("site_name").cloned().unwrap_or_default(),
        page_title => page_title,
        message => message.unwrap_or_default(),
        has_message => message.is_some(),
        logged_in => logged_in,
    }
}

fn render(state: &AppState, name: &str, ctx: Value) -> Response {
    match state
        .env
        .get_template(name)
        .and_then(|t| t.render(ctx))
    {
        Ok(html) => axum::response::Html(html).into_response(),
        Err(err) => {
            tracing::error!("模板 {name} 渲染失败: {err}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------- 首页

pub async fn handler_dashboard(State(state): State<AppState>, Query(flash): Query<Flash>) -> Response {
    let (total, drafts) = state.db.count_all_posts().await.unwrap_or((0, 0));
    let ctx = context! {
        total => total,
        drafts => drafts,
        published => total - drafts,
        recent => post_rows(&state.db.list_posts(5, 0).await.unwrap_or_default()),
        ..ctx_admin(&state, "仪表盘", flash_message(&flash), true).await,
    };
    render(&state, "admin/dashboard.jinja", ctx)
}

// ---------------------------------------------------------------- 站点信息

pub async fn handler_settings(State(state): State<AppState>, Query(flash): Query<Flash>) -> Response {
    let settings = state.db.all_settings().await.unwrap_or_default();
    let get = |key: &str| settings.get(key).cloned().unwrap_or_default();
    let ctx = context! {
        form => context! {
            site_name => get("site_name"),
            site_description => get("site_description"),
            site_url => get("site_url"),
            author => get("author"),
            about_content => get("about_content"),
        },
        action_url => admin_url(&state, "/settings"),
        password_url => admin_url(&state, "/password"),
        ..ctx_admin(&state, "站点设置", flash_message(&flash), true).await,
    };
    render(&state, "admin/settings.jinja", ctx)
}

#[derive(Deserialize)]
pub struct SettingsForm {
    site_name: String,
    site_description: String,
    site_url: String,
    author: String,
    about_content: String,
}

pub async fn handler_settings_save(
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Response {
    let pairs = [
        ("site_name", form.site_name.as_str()),
        ("site_description", form.site_description.as_str()),
        ("site_url", form.site_url.as_str()),
        ("author", form.author.as_str()),
        ("about_content", form.about_content.as_str()),
    ];
    match state.db.set_settings(&pairs).await {
        Ok(()) => Redirect::to(&admin_url(&state, "/settings?saved=1")).into_response(),
        Err(err) => {
            tracing::error!("保存站点设置失败: {err}");
            Redirect::to(&admin_url(&state, "/settings?err=db")).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct PasswordForm {
    current: String,
    next: String,
    confirm: String,
}

pub async fn handler_password_change(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<PasswordForm>,
) -> Response {
    let stored = state.db.get_setting("admin_password").await.unwrap_or_default();
    let back = |reason: &str| {
        Redirect::to(&admin_url(&state, &format!("/settings?err={reason}"))).into_response()
    };

    if !verify_password(&form.current, &stored) {
        return back("password");
    }
    if form.next != form.confirm || form.next.is_empty() {
        return back("mismatch");
    }
    let hash = match hash_password(&form.next) {
        Ok(hash) => hash,
        Err(err) => {
            tracing::error!("密码哈希失败: {err}");
            return back("db");
        }
    };
    if state.db.set_settings(&[("admin_password", &hash)]).await.is_err() {
        return back("db");
    }
    // 改密后强制所有会话重新登录
    let _ = state.db.clear_all_sessions().await;
    let jar = jar.remove(session_cookie(&state, ""));
    (jar, Redirect::to(&login_url(&state))).into_response()
}

// ---------------------------------------------------------------- 文章列表

fn post_rows(posts: &[Post]) -> Vec<Value> {
    posts
        .iter()
        .map(|p| {
            context! {
                id => p.id,
                title => p.title.clone(),
                slug => p.slug.clone(),
                date => p.datetime.format("%Y-%m-%d").to_string(),
                updated => p.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                published => p.published != 0,
                chars => p.content.chars().count(),
            }
        })
        .collect()
}

pub async fn handler_posts(State(state): State<AppState>, Query(flash): Query<Flash>) -> Response {
    let page = flash.page.unwrap_or(1).max(1);
    let offset = (page - 1) * PER_PAGE;
    let (total, _) = state.db.count_all_posts().await.unwrap_or((0, 0));
    let posts = state.db.list_posts(PER_PAGE, offset).await.unwrap_or_default();
    let pages = (total / PER_PAGE) + if total % PER_PAGE > 0 { 1 } else { 0 };
    let ctx = context! {
        posts => post_rows(&posts),
        page => page,
        pages => pages.max(1),
        total => total,
        new_url => admin_url(&state, "/posts/new"),
        ..ctx_admin(&state, "文章", flash_message(&flash), true).await,
    };
    render(&state, "admin/posts.jinja", ctx)
}

// ---------------------------------------------------------------- 文章编辑

#[derive(Deserialize)]
pub struct PostForm {
    title: String,
    slug: String,
    description: String,
    content: String,
    datetime: String,
    action: Option<String>,
}

struct EditModel {
    id: i64,
    slug: String,
    title: String,
    description: String,
    content: String,
    datetime_local: String,
    published: bool,
}

impl From<&Post> for EditModel {
    fn from(p: &Post) -> Self {
        Self {
            id: p.id,
            slug: p.slug.clone(),
            title: p.title.clone(),
            description: p.description.clone(),
            content: p.content.clone(),
            datetime_local: p.datetime.format("%Y-%m-%dT%H:%M").to_string(),
            published: p.published != 0,
        }
    }
}

fn empty_model() -> EditModel {
    EditModel {
        id: 0,
        slug: String::new(),
        title: String::new(),
        description: String::new(),
        content: String::new(),
        datetime_local: Local::now().naive_local().format("%Y-%m-%dT%H:%M").to_string(),
        published: false,
    }
}

async fn edit_ctx(state: &AppState, model: &EditModel, message: Option<&str>) -> Value {
    let action_tail = if model.id == 0 {
        "/posts/new".to_string()
    } else {
        format!("/posts/{}", model.id)
    };
    let preview_url = preview_url(state, model).await;
    context! {
        editing => context! {
            id => model.id,
            slug => model.slug.clone(),
            title => model.title.clone(),
            description => model.description.clone(),
            content => model.content.clone(),
            datetime_local => model.datetime_local.clone(),
            published => model.published,
        },
        is_new => model.id == 0,
        action_url => admin_url(state, &action_tail),
        back_url => admin_url(state, "/posts"),
        preview_url => preview_url,
        ..ctx_admin(state, if model.id == 0 { "新文章" } else { "编辑文章" }, message, true).await,
    }
}

// 草稿对外是 404, 因此只有已发布文章才给出前台链接
async fn preview_url(state: &AppState, model: &EditModel) -> String {
    if !model.published || model.slug.is_empty() {
        return String::new();
    }
    let site = state
        .db
        .get_setting("site_url")
        .await
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string();
    let year: String = model.datetime_local.chars().take(4).collect();
    format!("{site}/blog/{year}/{}", model.slug)
}

fn normalize_slug(raw: &str, datetime: &NaiveDateTime) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_dash = true;
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        // 中文标题转不出 ASCII slug, 回落到日期 + 随机后缀
        out = format!("{}-{}", datetime.format("%Y-%m-%d"), &random_token()[..8]);
    }
    out
}

fn parse_datetime(raw: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .ok()
}

fn validate(form: &PostForm) -> Option<&'static str> {
    if form.title.trim().is_empty() {
        return Some("empty");
    }
    if form.title.chars().count() > MAX_TITLE
        || form.description.chars().count() > MAX_DESCRIPTION
        || form.content.chars().count() > MAX_CONTENT
    {
        return Some("length");
    }
    None
}

fn desired_published(action: Option<String>, current: bool) -> bool {
    match action.as_deref() {
        Some("publish") => true,
        Some("unpublish") => false,
        _ => current,
    }
}

async fn to_new_post(state: &AppState, form: &PostForm, except_id: Option<i64>, current: bool) -> Result<(EditModel, NewPost), (EditModel, &'static str)> {
    let mut model = EditModel {
        id: except_id.unwrap_or(0),
        slug: form.slug.trim().to_string(),
        title: form.title.clone(),
        description: form.description.clone(),
        content: form.content.clone(),
        datetime_local: form.datetime.clone(),
        published: desired_published(form.action.clone(), current),
    };
    if let Some(reason) = validate(form) {
        return Err((model, reason));
    }
    let Some(datetime) = parse_datetime(&form.datetime) else {
        return Err((model, "datetime"));
    };
    let slug = normalize_slug(&form.slug, &datetime);
    if state.db.slug_exists(&slug, except_id).await.unwrap_or(false) {
        model.slug = slug;
        return Err((model, "slug"));
    }
    model.slug = slug.clone();
    let new = NewPost {
        slug,
        title: model.title.clone(),
        description: model.description.clone(),
        content: model.content.clone(),
        datetime,
        published: model.published,
    };
    Ok((model, new))
}

async fn invalid_form(state: &AppState, model: EditModel, reason: &str) -> Response {
    let ctx = edit_ctx(state, &model, Some(flash_text(reason))).await;
    render(state, "admin/post_edit.jinja", ctx)
}

pub async fn handler_post_new(State(state): State<AppState>) -> Response {
    let model = empty_model();
    let ctx = edit_ctx(&state, &model, None).await;
    render(&state, "admin/post_edit.jinja", ctx)
}

pub async fn handler_post_edit(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(flash): Query<Flash>,
) -> Response {
    let post = match state.db.post_by_id(id).await {
        Ok(Some(post)) => post,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("读取文章 {id} 失败: {err}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let model = EditModel::from(&post);
    let ctx = edit_ctx(&state, &model, flash_message(&flash)).await;
    render(&state, "admin/post_edit.jinja", ctx)
}

pub async fn handler_post_create(
    State(state): State<AppState>,
    Form(form): Form<PostForm>,
) -> Response {
    let (model, new) = match to_new_post(&state, &form, None, false).await {
        Ok(pair) => pair,
        Err((model, reason)) => return invalid_form(&state, model, reason).await,
    };
    match state.db.insert_post(&new).await {
        Ok(id) => {
            Redirect::to(&admin_url(&state, &format!("/posts/{id}?saved=1"))).into_response()
        }
        Err(err) => {
            tracing::error!("新建文章失败: {err}");
            invalid_form(&state, model, "db").await
        }
    }
}

pub async fn handler_post_update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<PostForm>,
) -> Response {
    let current = match state.db.post_by_id(id).await {
        Ok(Some(post)) => post.published != 0,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("读取文章 {id} 失败: {err}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let (model, new) = match to_new_post(&state, &form, Some(id), current).await {
        Ok(pair) => pair,
        Err((model, reason)) => return invalid_form(&state, model, reason).await,
    };
    match state.db.update_post(id, &new).await {
        Ok(true) => {
            Redirect::to(&admin_url(&state, &format!("/posts/{id}?saved=1"))).into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("更新文章 {id} 失败: {err}");
            invalid_form(&state, model, "db").await
        }
    }
}

fn flash_text(reason: &str) -> &'static str {
    flash_message(&Flash {
        err: Some(reason.to_string()),
        ..Default::default()
    })
    .unwrap_or("操作未完成")
}

async fn set_published(state: &AppState, id: i64, published: bool) -> Response {
    match state.db.set_post_published(id, published).await {
        Ok(true) => Redirect::to(&admin_url(&state, "/posts?saved=1")).into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("切换文章 {id} 发布状态失败: {err}");
            Redirect::to(&admin_url(&state, "/posts?err=db")).into_response()
        }
    }
}

pub async fn handler_post_publish(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    set_published(&state, id, true).await
}

pub async fn handler_post_unpublish(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    set_published(&state, id, false).await
}

pub async fn handler_post_delete(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    match state.db.delete_post(id).await {
        Ok(true) => Redirect::to(&admin_url(&state, "/posts?saved=1")).into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("删除文章 {id} 失败: {err}");
            Redirect::to(&admin_url(&state, "/posts?err=db")).into_response()
        }
    }
}
