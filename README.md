# Kblog

一个自带后台的个人博客引擎：Rust + Axum + MiniJinja + SQLite。

[English](README.md) · [中文说明](README_zh.md)

## 这是什么

单进程的博客服务。前台是纯服务端渲染的静态风格页面（文章归档、碎念、关于），后台在同一个进程里，负责站点设置、账号密码和文章的增删改与发布。没有前端构建步骤，没有 JS 框架，数据库就是一个 SQLite 文件。

## 特性

- **服务端渲染**：MiniJinja 模板 + 按页面拆分的 CSS，深浅色模式跟随系统 `prefers-color-scheme`
- **内置后台**：登录、仪表盘、站点设置、文章列表（分页 20 条）/ 新建 / 编辑 / 发布 / 转草稿 / 删除
- **草稿与发布**：`published` 字段控制，草稿在归档页和详情页都是 404，不会出现「登录后预览」这类半成品逻辑
- **Markdown 正文**：表格、脚注、删除线、任务列表（`pulldown-cmark`）
- **自动生成 slug**：转小写、非 ASCII 字母数字折叠为 `-`；标题转不出可用 slug 时回落为 `{日期}-{8位十六进制}`
- **零外部依赖部署**：只需一个二进制 + SQLite 文件 + `templates/` 与 `assets/` 两个目录
- **响应压缩**：gzip / brotli（`tower-http` CompressionLayer）

## 快速开始

需要 Rust 1.85 以上（`Cargo.toml` 使用 `edition = "2024"`）。

```bash
git clone https://github.com/korbinZhang/kblog.git
cd kblog
cargo run
```

首次启动会在日志里打印一次随机生成的 12 位初始密码：

```
INFO kblog::admin: 已生成后台初始密码: <12位随机码>
INFO kblog:        后台登录页: http://127.0.0.1:3000/admin/login
```

打开 <http://127.0.0.1:3000> 看前台，`/admin/login` 进后台。**登录后请先到「站点设置」修改密码**——改密会让所有已登录会话失效。

发布构建用 `cargo build --release`，产物在 `target/release/kblog`（crate 名就是 `kblog`）。

## 配置

全部配置来自环境变量，启动时通过 `dotenvy` 读取同目录 `.env`（可选）。可以先从模板复制一份：

```bash
cp .env.example .env
```

某一项未设置时使用 `src/config.rs` 里的内置默认值。

| 变量 | 默认值 | 说明 |
|---|---|---|
| `WEB_ADDR` | `127.0.0.1` | 监听地址 |
| `WEB_PORT` | `3000` | 监听端口 |
| `TEMPLATES_PATH` | `templates` | 模板目录 |
| `ASSETS_PATH` | `assets` | 静态目录，由 fallback 直接提供 |
| `DATABASE_PATH` | `data/data.db` | SQLite 文件，不存在时自动创建 |
| `ADMIN_PATH` | `admin` | 后台路径前缀，即 `/{ADMIN_PATH}/login` |

`ADMIN_PATH` 的校验规则：非空、≤ 64 字节、仅允许 ASCII 字母数字与 `-` `_`、且不能与公开路径 `blog` / `thinking` / `about` 冲突。不满足时打印 `WARN` 并回落到 `admin`。改这个值需要重启服务，旧地址会立刻整体 404。

站点标题、描述、URL、「关于」正文等运营信息存在数据库 `settings` 表，由后台页面维护，不占环境变量。密码与会话也在库里，同样不经过 `Config`（`Config::init()` 会完整打印配置结构体，敏感值不能放进去）。

## 路由

前台：

| 路径 | 说明 |
|---|---|
| `/`、`/blog`、`/blog/{year}` | 文章年度归档，缺省年份取最新一篇 |
| `/blog/{year}/{slug}` | 文章详情 |
| `/thinking`、`/thinking/{year}` | 碎念列表 |
| `/about` | 关于页 |
| `/{ADMIN_PATH}/login` | 后台登录 |

后台（前缀 `/{ADMIN_PATH}`）：

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| GET / POST | `/login` | 公开 | 登录 |
| POST | `/logout` | 公开 | 登出 |
| GET | `/` | 需要 | 仪表盘：总数 / 已发布 / 草稿 + 最近 5 篇 |
| GET / POST | `/settings` | 需要 | 站点信息 |
| POST | `/password` | 需要 | 修改密码 |
| GET | `/posts` | 需要 | 文章列表，`?page=` 分页 |
| GET / POST | `/posts/new` | 需要 | 新建 |
| GET / POST | `/posts/{id}` | 需要 | 编辑，按提交的按钮决定发布状态 |
| POST | `/posts/{id}/publish` · `/unpublish` | 需要 | 发布 / 转草稿 |
| POST | `/posts/{id}/delete` | 需要 | 删除，仅前端 `confirm()` |

未登录访问任何需要鉴权的路径一律返回 **404**（不是 302 跳转登录），让后台与不存在的路径不可区分。

## 项目结构

```
src/
  main.rs        路由与前台 handler、AppState
  config.rs      环境变量读取与 ADMIN_PATH 校验
  db.rs          SQLite 建表与全部读写查询
  admin.rs       后台路由、会话、argon2 密码
  markdown.rs    pulldown-cmark 配置
templates/       base / blog / thinkings / post / about + admin/ 下 6 个后台模板
assets/          base / archive / blog / thinkings / post / admin 六个 CSS 与图标
data/            SQLite 数据文件（gitignore）
```

## 数据模型

`src/db.rs::create_tables()` 用裸 DDL 建表，无迁移框架：

```
settings(key PK, value)              site_name / site_description / site_url / author / about_content / admin_password
posts(id PK, slug UK, title, description, content, published, datetime, updated_at)
thinkings(id PK, datetime, content)  公开页「碎念」
admin_sessions(token PK, created_at, expires_at)
```

长度上限：标题 200、摘要 500、正文 200 000 字符。校验失败会带错误 banner 重新渲染表单，不丢已输入内容。

## 安全设计

已处理：模板开启 HTML 自动转义（MiniJinja 默认不认 `.jinja` 后缀，必须显式打开，否则后台写入的标题会成为持久化 XSS）；密码用 argon2id 哈希 + 常数时间校验；会话是服务端 token 表，7 天过期，Cookie 带 `HttpOnly` + `SameSite=Lax` + `Path=/{ADMIN_PATH}`，登出与改密码可撤销；所有写操作都是 POST；列表查询带 `LIMIT/OFFSET`，表单有长度上限。

已知边界（有意接受，前提是只有单一可信管理员）：正文里的原始 HTML 不过滤——若引入多作者或不可信投稿，必须加 `ammonia` 之类白名单消毒；无 CSRF token，依赖「写操作全 POST + SameSite=Lax」；Cookie 未设 `Secure`，因为本地跑在 http 下，上 HTTPS 时必须补；会话 token 明文入库；删除不可撤销；初始密码会打印一次到日志。

## 已知限制

- 「碎念」有公开页但没有后台，目前只能直接写库
- 草稿没有预览功能，想看效果只能先发布
- 没有 RSS / 站点地图 / 评论 / 标签分类
- 无自动化测试；前台行为靠手工验证

发现安全问题时请勿公开提 issue，直接联系维护者。

## 许可

[MIT](LICENSE) © 2026 korbin
