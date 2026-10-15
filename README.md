# AI 内容工厂

Vue 3 + Element Plus + Tauri 2 桌面端。通过内置 Node 运行 `catbus-cli` 完成小红书 / 抖音登录与内容搜索；本地 SQLite 管理内容库、草稿与竞品；Rust 启动 Chrome/Edge CDP；Node Playwright 拟人化填表发布；OpenAI 兼容大模型驱动草稿 AI / 竞品助手。

## 环境

- Node.js ≥ 22.22（开发机）
- Rust / Cargo（Tauri）
- macOS / Windows / Linux
- 本机已安装 Chrome 或 Edge（CDP 发布）

## 安装

```bash
npm install
cd resources/catbus && npm install && cd ../..
cd resources/cdp && npm install && cd ../..
npm run fetch-node
```

## 开发

```bash
npm run tauri:dev
```

## 功能导航

**小红书**：账号矩阵（搜索账号 + 桌面 CDP）→ 笔记发现 → 内容库 → 竞品分析（笔记监控 + 用户监控）→ 草稿工坊 → 发布中心  

**抖音**：账号矩阵 → 作品发现 → 内容库 → 草稿工坊  

**系统设置**：环境诊断 | 大模型配置

## 冒烟建议

1. 设置 → 环境诊断：Node / catbus 就绪  
2. 大模型配置：添加供应商 → 测试连通 → 设为默认  
3. 账号矩阵 → 桌面 CDP：选浏览器路径并保存 → 启动 Profile → 登录创作者中心  
4. 草稿工坊：新建 / AI 改写 → 保存配图 URL  
5. 发布中心：选草稿 + 在线 Profile →「CDP 填表」（先不点发布）  
6. 竞品分析：粘贴笔记链接 → 刷新 → 看趋势 / 助手

## 说明

- `CATBUS_HOME` 指向应用数据目录下的 `catbus/`，不写 `~/.catbus`
- CDP 拟人化：贝塞尔鼠标、逐字输入、分级停顿、顶部「AI正在操作浏览器」横幅；不复用数字员工 Python
- LLM 密钥仅存本机 SQLite，业务请求时由前端携带 `llm` payload 调用 Node
- DeepSeek 预设对齐官方文档：`base_url=https://api.deepseek.com`，模型 `deepseek-flash` / `deepseek-v4-pro`（见 [DeepSeek API 文档](https://api-docs.deepseek.com/zh-cn/)）
- 合规使用：遵守各平台条款，控制频率，勿用于违法用途
