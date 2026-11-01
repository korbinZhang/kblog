use super::config::Config;
use chrono::{Local, NaiveDateTime};
use rand::RngExt;
use sqlx::{FromRow, Pool, Sqlite, sqlite::SqlitePoolOptions};
use std::collections::HashMap;

#[derive(Clone, FromRow)]
pub struct Post {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub content: String,
    pub published: i64,
    pub datetime: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

pub struct NewPost {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub content: String,
    pub datetime: NaiveDateTime,
    pub published: bool,
}

#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    pub async fn init(config: &Config) -> Self {
        let pool = SqlitePoolOptions::new()
            .connect(&format!("sqlite:{}?mode=rwc", config.database_path))
            .await
            .expect("数据库连接失败");

        tracing::info!("数据库连接成功");
        let db = Self { pool };
        db.create_tables().await;
        tracing::info!("数据库初始化完成");
        db
    }

    async fn create_tables(&self) {
        let defaults = vec![
            ("site_name", "Korbin的小站"),
            ("author", "Korbin"),
            ("site_description", "网站描述"),
            ("site_url", "http://127.0.0.1:3000"),
            (
                "about_content",
                "<h2>关于我</h2>\n<p>这是一个关于页面。</p>",
            ),
        ];

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        )
        .execute(&self.pool)
        .await
        .unwrap();

        for (key, value) in defaults {
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO NOTHING",
            )
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .unwrap();
        }

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS thinkings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            datetime DATETIME NOT NULL,
            content TEXT NOT NULL
        )",
        )
        .execute(&self.pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_thinkings_datetime ON thinkings(datetime DESC)",
        )
        .execute(&self.pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS posts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            slug TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL,
            published INTEGER NOT NULL DEFAULT 0,
            datetime DATETIME NOT NULL,
            updated_at DATETIME NOT NULL
        )",
        )
        .execute(&self.pool)
        .await
        .unwrap();

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_posts_datetime ON posts(datetime DESC)")
            .execute(&self.pool)
            .await
            .unwrap();

        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_posts_slug ON posts(slug)")
            .execute(&self.pool)
            .await
            .unwrap();

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_posts_published ON posts(published, datetime DESC)")
            .execute(&self.pool)
            .await
            .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS admin_sessions (
            token TEXT PRIMARY KEY,
            created_at DATETIME NOT NULL,
            expires_at DATETIME NOT NULL
        )",
        )
        .execute(&self.pool)
        .await
        .unwrap();
    }

    pub async fn year_counts(&self) -> Vec<(u16, i64)> {
        sqlx::query_as(
            "SELECT CAST(strftime('%Y', datetime) AS INTEGER) AS year,
                    COUNT(*) AS count
             FROM thinkings
             GROUP BY year
             ORDER BY year DESC",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    pub async fn thinking_by_year(&self, year: u16) -> Option<Vec<(NaiveDateTime, String)>> {
        let rows = sqlx::query_as(
            "SELECT datetime, content FROM thinkings
             WHERE strftime('%Y', datetime) = ?
             ORDER BY datetime DESC",
        )
        .bind(year.to_string())
        .fetch_all(&self.pool)
        .await
        .unwrap();
        if rows.is_empty() { None } else { Some(rows) }
    }

    pub async fn latest_year(&self) -> Option<u16> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT CAST(strftime('%Y', datetime) AS INTEGER) FROM thinkings
             ORDER BY datetime DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .unwrap();
        row.map(|(y,)| y as u16)
    }

    pub async fn post_year_counts(&self) -> Vec<(u16, i64)> {
        sqlx::query_as(
            "SELECT CAST(strftime('%Y', datetime) AS INTEGER) AS year,
                    COUNT(*) AS count
             FROM posts
             WHERE published = 1
             GROUP BY year
             ORDER BY year DESC",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    pub async fn posts_by_year(
        &self,
        year: u16,
    ) -> Option<Vec<(NaiveDateTime, String, String, String, String)>> {
        let rows = sqlx::query_as(
            "SELECT datetime, title, slug, description, content FROM posts
             WHERE published = 1 AND strftime('%Y', datetime) = ?
             ORDER BY datetime DESC",
        )
        .bind(year.to_string())
        .fetch_all(&self.pool)
        .await
        .unwrap();
        if rows.is_empty() { None } else { Some(rows) }
    }

    pub async fn latest_post_year(&self) -> Option<u16> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT CAST(strftime('%Y', datetime) AS INTEGER) FROM posts
             WHERE published = 1
             ORDER BY datetime DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .unwrap();
        row.map(|(y,)| y as u16)
    }

    pub async fn post_by_slug(
        &self,
        slug: &str,
    ) -> Option<(NaiveDateTime, String, String, String, String)> {
        sqlx::query_as(
            "SELECT datetime, title, slug, description, content FROM posts
             WHERE published = 1 AND slug = ?",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }

    pub async fn get_setting(&self, key: &str) -> Option<String> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .unwrap();
        row.map(|(v,)| v)
    }

    pub async fn all_settings(&self) -> sqlx::Result<HashMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM settings").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn set_settings(&self, pairs: &[(&str, &str)]) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        for (key, value) in pairs {
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES (?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    pub async fn create_session(&self, token: &str, ttl_days: i64) -> sqlx::Result<()> {
        let now = now();
        sqlx::query("INSERT INTO admin_sessions (token, created_at, expires_at) VALUES (?, ?, ?)")
            .bind(token)
            .bind(now)
            .bind(now + chrono::Duration::days(ttl_days))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn session_is_valid(&self, token: &str) -> sqlx::Result<bool> {
        let now = now();
        sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= ?")
            .bind(now)
            .execute(&self.pool)
            .await?;
        let row: Option<(String,)> =
            sqlx::query_as("SELECT token FROM admin_sessions WHERE token = ?")
                .bind(token)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn delete_session(&self, token: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM admin_sessions WHERE token = ?")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn clear_all_sessions(&self) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM admin_sessions")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_posts(&self, limit: i64, offset: i64) -> sqlx::Result<Vec<Post>> {
        sqlx::query_as(
            "SELECT id, slug, title, description, content, published, datetime, updated_at
             FROM posts ORDER BY datetime DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn count_all_posts(&self) -> sqlx::Result<(i64, i64)> {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN published = 0 THEN 1 ELSE 0 END), 0)
             FROM posts",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or((0, 0)))
    }

    pub async fn post_by_id(&self, id: i64) -> sqlx::Result<Option<Post>> {
        sqlx::query_as(
            "SELECT id, slug, title, description, content, published, datetime, updated_at
             FROM posts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn insert_post(&self, post: &NewPost) -> sqlx::Result<i64> {
        let now = now();
        let result = sqlx::query(
            "INSERT INTO posts (slug, title, description, content, published, datetime, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&post.slug)
        .bind(&post.title)
        .bind(&post.description)
        .bind(&post.content)
        .bind(post.published as i64)
        .bind(post.datetime)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn update_post(&self, id: i64, post: &NewPost) -> sqlx::Result<bool> {
        let result = sqlx::query(
            "UPDATE posts SET slug = ?, title = ?, description = ?, content = ?,
                    published = ?, datetime = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&post.slug)
        .bind(&post.title)
        .bind(&post.description)
        .bind(&post.content)
        .bind(post.published as i64)
        .bind(post.datetime)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_post_published(&self, id: i64, published: bool) -> sqlx::Result<bool> {
        let result = sqlx::query("UPDATE posts SET published = ? WHERE id = ?")
            .bind(published as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_post(&self, id: i64) -> sqlx::Result<bool> {
        let result = sqlx::query("DELETE FROM posts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn slug_exists(&self, slug: &str, except_id: Option<i64>) -> sqlx::Result<bool> {
        let row: Option<(i64,)> = match except_id {
            Some(id) => {
                sqlx::query_as("SELECT id FROM posts WHERE slug = ? AND id <> ?")
                    .bind(slug)
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_as("SELECT id FROM posts WHERE slug = ?")
                    .bind(slug)
                    .fetch_optional(&self.pool)
                    .await?
            }
        };
        Ok(row.is_some())
    }
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

pub fn random_token() -> String {
    let mut rng = rand::rng();
    let a: u128 = rng.random();
    let b: u128 = rng.random();
    format!("{a:032x}{b:032x}")
}

pub fn random_password() -> String {
    // 去掉易混淆的 i l 1 o 0
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::rng();
    (0..12)
        .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
        .collect()
}
