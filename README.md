# pibipu (alias: ⚡嘀嘀嘀⚡) ![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?style=for-the-badge&logo=discord&logoColor=white) ![Threads](https://img.shields.io/badge/Threads-000000.svg?style=for-the-badge&logo=threads&logoColor=white) ![YouTube](https://img.shields.io/badge/YouTube-%23FF0000.svg?style=for-the-badge&logo=YouTube&logoColor=white) ![Rust](https://img.shields.io/badge/Rust-000000.svg?style=for-the-badge&logo=rust&logoColor=white)

![pibipu, a.k.a arisu dance.](./public/pibipu.gif)

[社會民主黨官方Discord](https://discord.gg/f4D4vvUG2w) 的 Threads / YouTube 通知機器人。Rust + [serenity](https://github.com/serenity-rs/serenity)，跑在 [Fly.io](https://fly.io)（arn）。

## 它做什麼

- **Threads**：直接讀 `threads.com/@user` 和 `/@user/replies` 頁面裡內嵌的 JSON（跟 RSSHub 的做法一樣，但不經過 RSSHub）。本人發文、引用、回覆都會通知；轉發別人的貼文不會。訊息只放 [fixthreads](https://fixthreads.seria.moe) 網址，embed 由 Discord 自動產生。
- **YouTube**（Data API v3）：影片上架、直播預定、開播、結束四種通知。預定時同時在伺服器建立一個 Discord 活動，改時間會跟著改，開播設 Active，結束設 Completed。停機期間或兩次輪詢之間新增且已結束的直播，會補發一次結束通知。
- 狀態存在 Fly volume 的 `/data/state.json`。**第一次啟動**或新增來源時，以各來源第一次成功抓取的資料建立基線，不發舊通知、不建活動；既有直播仍會追蹤之後的開播與結束。之後重啟會補發停機期間漏掉的（YouTube 最新 10 筆，Threads 目前頁面可見貼文）。
- 通知先存入狀態檔，成功送出的目的頻道個別移除；暫時故障或欠缺送訊權限會重試，同頻道維持順序。活動建立暫時失敗也會重試；欠缺活動權限時則記錄錯誤、照常送通知。損壞或無法寫入狀態檔會讓程式退出，讓 Fly 的重啟策略介入。

## 設定 `config.json`

包進 image，改完 `fly deploy`。

目前已設定的對應：

| Discord 頻道 | YouTube 頻道 | Threads |
| --- | --- | --- |
| `1360121904526135394` | `UCICZqWqYDD4zfwQ9_7Kw-2g` | `sdparty.tw` |
| `1360132852959936594` | `UC_HqHm72u8efzD_kij0P8PQ` | `miaopoyatw`、`fz.hikari.lee`（各自使用不同前綴） |

```json
{
  "guild_id": "伺服器ID",
  "threads_interval_secs": 300,
  "youtube_interval_secs": 60,
  "targets": [
    {
      "channel_id": "頻道ID",
      "threads": ["miaopoyatw"],
      "youtube": ["UCxxxxxxxxxxxxxxxxxxxxxx"],
      "prefix": {
        "threads": "",
        "video": "<@&角色ID> 看啦看啦！",
        "scheduled": "",
        "live": "<@&角色ID> 開播了",
        "end": ""
      }
    },
    {
      "channel_id": "頻道ID",
      "threads": ["fz.hikari.lee"],
      "prefix": {
        "threads": "🐰早安，您的復中里里長小光已上線："
      }
    }
  ]
}
```

- 每個 target 指定一個 Discord 頻道、一組來源與通知前綴。同一個 `channel_id` 可以出現在多個 target，讓不同來源使用不同前綴（如上例）。
- 如果多個來源共用前綴，也可放在同一個 target，例如 `"threads": ["miaopoyatw", "fz.hikari.lee"]`。
- 同一來源可以通知不同 Discord 頻道；同一平台的同一來源不得在同一個 Discord 頻道重複訂閱（包含同一 target 內重複填寫）。Threads 帳號會先去除前後空白、開頭的 `@` 並轉成小寫，再檢查重複。
- ID 不得為 0，輪詢間隔必須大於 0。Threads 可填 `@username`，YouTube 需填 24 字元的 `UC` 頻道 ID。
- `prefix` 可以是一個字串（所有訊息都加）或依訊息種類的物件，缺的種類就不加。
- 訊息長相：上架／開播＝prefix + 影片網址；預定＝`📅 直播預定 <t:…:F>（<t:…:R>）` + 影片網址 + 活動連結；結束＝`📺 直播結束 ｜ 🕑 時長 ｜ 👀 觀看 ｜ ❤️ 按讚` + 網址。

環境變數：`DISCORD_TOKEN`、`GOOGLE_API_KEY`（必填）；`CONFIG_PATH`（預設 `config.json`）、`STATE_PATH`（預設 `/data/state.json`）。

Bot 在目標頻道需有 View Channel、Send Messages、Embed Links；外部活動需有伺服器層級的 **Create Events**（建立活動）與 **Manage Events**（管理活動）。權限在 Discord 伺服器的角色設定中授予；不需要任何 privileged intent。[Discord 官方活動文件](https://docs.discord.com/developers/resources/guild-scheduled-event)

## 本機

Rust 1.88 以上（鎖定依賴的最低需求）。

```bash
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

離線測試包含真實 serenity HTTP 呼叫對接本機模擬伺服器，檢查失敗重送、重啟、活動共用及欠缺活動權限。以下公開 Threads 抓取測試另外執行：

```bash
cargo test --locked threads::tests::live_fetch -- --ignored --exact
```

YouTube API 格式測試需先設定 `GOOGLE_API_KEY`，會消耗 2 單位配額；不會向 Discord 發送訊息：

```bash
cargo test --locked youtube::tests::live_shapes -- --ignored --exact
```

本機啟動（PowerShell，會實際監聽並發送通知）：

```powershell
$env:DISCORD_TOKEN = "..."
$env:GOOGLE_API_KEY = "..."
$env:STATE_PATH = "state.json"
cargo run --locked
```

Podman 建置與本機啟動（兩個 secret 從目前環境傳入，狀態存在 named volume）：

```powershell
podman machine start # 機器尚未啟動時
podman build -t localhost/pibipu:local .
podman volume create pibipu_data
podman run --rm --name pibipu-local -e DISCORD_TOKEN -e GOOGLE_API_KEY -v pibipu_data:/data localhost/pibipu:local
```

## 部署到 Fly.io

第一次（本機沒裝 flyctl 的話先裝）：

```powershell
pwsh -Command "iwr https://fly.io/install.ps1 -useb | iex"
```

```bash
fly auth login
```

舊的 Bun 版如果還有兩台機器，先縮到一台，不然一台機器要一顆 volume：

```bash
fly scale count 1 --app pibipu
```

建 volume（名字要跟 `fly.toml` 的 `[mounts].source` 一樣）：

```bash
fly volumes create pibipu_data --app pibipu --region arn --size 1 --yes
```

設 secret（既有 `DISCORD_TOKEN` 可沿用；新 app 需一併 `fly secrets set DISCORD_TOKEN="..." --app pibipu --stage`）、拿掉舊的 Mongo 連線：

```bash
fly secrets set GOOGLE_API_KEY="..." --app pibipu --stage
```

```bash
fly secrets unset MONGODB_URI --app pibipu --stage
```

部署（`--ha=false` 只開一台）：

```bash
fly deploy --app pibipu --ha=false
```

看 log：

```bash
fly logs --app pibipu
```

之後改設定或程式都只要 `fly deploy`。

## 已知限制與未納入範圍

- 外部活動依原規格設定為開始後 3 小時結束；Discord 會自動完成活動，超過 3 小時的直播仍能通知結束，但不會自動延長活動卡片。
- 來源各記住最近 200 個 ID；未觀察到結束的直播在「首次發現／預定／開播」最晚時間再過 7 天後移除。刪除直播不會取消 Discord 活動。
- `state.json` 的 pending 通知會持久保存，直到送出、或遇到不可修復的資料錯誤（例如超長訊息、已刪除頻道）並記錄後丟棄。權限長期未修復時佇列會持續增加。
- Discord 已接受請求、但回應遺失或尚未保存本機進度就斷電，重試仍可能造成重複；本機 JSON 與外部 API 之間沒有跨系統交易。請只運行一個 bot 實例。
- Threads 依賴公開頁面資料，登入牆或頁面格式變更會記錄錯誤並於下一輪重試。

Facebook、自訂 embed 樣式、活動封面圖、直播被刪時取消活動、往回分頁補發、slash 指令、資料庫。要加來源的話：`config.rs` 加欄位、寫一個 `fetch`、在 `main.rs` 的迴圈多一輪。

Made with ❤️ ＆ 🌹, and 🦀
