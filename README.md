# p1505-duplex

讓接在 Linux CUPS 伺服器上的 **HP LaserJet P1505** 也能雙面列印：正面先印，把紙放回紙匣，按「繼續」，再印背面。卡紙或一次進了兩張紙時，可以只補印出問題的那幾張。

包含兩個部分：

- **`server/`**：跑在列印伺服器上的 Rust 程式。負責拆頁、提供網頁與 JSON API，並在印表機開機時自動載入韌體。
- **`macos/`**：macOS 選單列小程式（Swift / SwiftUI）。該翻面時跳出通知，通知上直接就有「繼續列印背面」按鈕；網頁上能做的事它都能做，還能查看印表機狀態與列印紀錄。

## 為什麼需要這個

P1505 **沒有雙面單元**。Windows / macOS 上 HP 原廠驅動的「雙面列印」其實是在電腦端完成的：先送奇數頁，跳出視窗請你把紙翻面放回紙匣，按下繼續後再送偶數頁。

印表機接在 CUPS 伺服器上時，電腦只是把整份檔案交給伺服器，沒有人能在你的電腦上跳出視窗，這個功能就不見了。這個專案在伺服器上把同樣的流程做回來，再用網頁和選單列小程式取代那個視窗。

另外，P1505 每次斷電都會遺失韌體，沒有韌體時收到的資料會被直接丟掉，CUPS 卻照樣標成「已完成」。HPLIP 會在插上時嘗試載入韌體，但如果佇列裡已經有工作在等，CUPS 會先搶走裝置，載入就因為 `Device busy` 失敗。

## 運作方式

```
電腦 ── 列印到「P1505 手動雙面」
          │
          ▼
CUPS 佇列 P1505_Duplex（Generic PDF PPD，先轉成 PDF）
          │
          ▼
duplex 後端（p1505-duplex）
  ├─ 保存原始 PDF（供補印，保留 24 小時）
  ├─ 正面：第 1、3、5… 頁 → 真正的印表機佇列，立即列印
  └─ 背面：第 2、4… 頁 → 同一個佇列，先保留（hold）
       · 依設定倒序、旋轉 180°
       · 頁數為奇數時補一張空白頁，讓紙疊對齊
          │
          ▼
http://<伺服器>/duplex/          網頁
http://<伺服器>/duplex/api/…     JSON API 與即時事件（選單列小程式使用）
  按「繼續列印背面」→ 釋放保留的工作
  「出問題了？」→ 補印第 N–M 張的正面或背面
```

伺服器端是一支執行檔，依呼叫方式扮演三種角色：

| 角色 | 呼叫方式 | 以誰的身分執行 |
| --- | --- | --- |
| CUPS 後端 | CUPS 執行 `/usr/lib/cups/backend/duplex` | `lp` |
| 網頁與 API | `p1505-duplex web`（systemd） | `p1505-duplex`（`lpadmin` 群組） |
| 韌體載入 | `p1505-duplex firmware`（udev → systemd） | `root` |

### 和 CUPS 溝通：IPP over domain socket

所有 CUPS 操作都直接透過 IPP，經由 `/run/cups/cups.sock` 進行，不呼叫 `lp`、`lpstat` 等指令、也不解析它們的輸出：

- 在 domain socket 上，cupsd 知道對方的 uid。管理操作（釋放、取消別人的工作、暫停佇列）帶上 `Authorization: PeerCred <使用者>`，只要該使用者屬於 CUPS 的 SystemGroup（`lpadmin`）就會通過，不需要密碼或憑證。
- 送出列印工作時不帶認證，工作的擁有者就是原本送件的使用者，而不是後端執行時的 `lp`。
- 工作狀態（排隊、保留、列印中、已停止、失敗、完成）和印表機狀態都直接讀 IPP 屬性，不受系統語系影響。

### PDF 處理

用 [lopdf](https://crates.io/crates/lopdf) 挑選頁面、重新排序、設定 `/Rotate`、產生空白頁。後端收到的 PDF 一定是 CUPS 的 `pdftopdf` 輸出，格式規整，執行時不需要 qpdf 或 Ghostscript。

## 伺服器

### 需求

- Debian / Ubuntu，CUPS 與 HPLIP，P1505 已能用 `hp:/usb/...` 單面列印
- HPLIP 專有外掛（`hp-plugin`），提供 `hp-firmware` 與 P1505 韌體檔
- `cups-filters` 的 `Generic-PDF_Printer-PDF.ppd`
- nginx（或任何能反向代理的網頁伺服器）
- 編譯：Docker（在 macOS 上可用 OrbStack），或本機 Rust 工具鏈

### 編譯

```bash
server/build.sh
```

在 Docker 的 `rust:alpine`（linux/amd64）裡編譯出靜態連結的 x86_64 執行檔：`server/target/linux/x86_64-unknown-linux-musl/release/p1505-duplex`。

執行測試：

```bash
cd server && cargo test
```

### 安裝

把執行檔、`server/dist/` 和 `server/install.sh` 複製到伺服器同一個目錄，以 root 執行：

```bash
sudo ./install.sh
```

`install.sh` 會：

- 把執行檔裝到 `/usr/local/bin/p1505-duplex`，並複製一份到 `/usr/lib/cups/backend/duplex`（Ubuntu 的 AppArmor 只允許 cupsd 執行後端目錄裡的檔案，所以不能用 symlink）
- 安裝 `/etc/cups/local.convs`、systemd 服務和 udev 規則
- 建立系統帳號 `p1505-duplex`（`lpadmin` 群組）與 `/var/spool/p1505-duplex`
- 建立共享佇列 `P1505_Duplex`，顯示名稱「P1505 手動雙面」
- 已存在的 `/etc/p1505-duplex.conf` 不會被覆蓋

最後參考 `server/dist/nginx.conf`，把 `/duplex/` 反向代理到 `127.0.0.1:8631`。即時事件的回應帶有 `X-Accel-Buffering: no`，nginx 不需要額外設定。

### 設定

`/etc/p1505-duplex.conf`（範例見 `server/dist/p1505-duplex.conf`）：

| 鍵 | 預設值 | 說明 |
| --- | --- | --- |
| `PRINTER` | `HP_LaserJet_P1505` | 真正印表機的 CUPS 佇列名稱 |
| `DUPLEX_QUEUE` | `P1505_Duplex` | 手動雙面佇列（測試頁與狀態查詢用） |
| `USB_ID` | `03f0:3f17` | 印表機的 USB vendor:product，用來判斷有沒有接上 |
| `EVEN_ORDER` | `reverse` | 背面的送出順序：`reverse` 或 `forward` |
| `EVEN_ROTATE` | `180` | 背面額外旋轉：`0` 或 `180` |
| `LISTEN` | `127.0.0.1:8631` | 網頁監聽位址 |
| `STATE_DIR` | `/var/spool/p1505-duplex` | 工作紀錄、原始 PDF 與韌體載入紀錄 |

後端每份工作都會重新讀取設定；網頁的設定改了之後要 `systemctl restart p1505-duplex-web`。每份工作會記下送出當時的順序與旋轉設定，補印時沿用，不受之後修改影響。

### 校正背面方向

背面的順序和方向取決於紙張路徑，以及你怎麼把紙疊放回紙匣。第一次使用時印一份測試頁（選單列小程式的右鍵選單也有「列印雙面測試頁」）：

```bash
p1505-duplex test-pdf
lp -d P1505_Duplex duplex-test-5p.pdf
```

測試頁每頁中央是大大的頁碼，上緣有「TOP page N」。印完後檢查：第 1 張的背面應該是正向的 2，第 2 張是 4，第 3 張的背面是空白。

| 現象 | 調整 |
| --- | --- |
| 頁序錯了（例如第 1 張的背面是 4） | 切換 `EVEN_ORDER` |
| 背面上下顛倒 | 切換 `EVEN_ROTATE` |

預設值是「正面印完的紙疊直接整疊放回紙匣」時實測出來的。

### 韌體自動載入

插上印表機或開機時，udev 啟動 `p1505-firmware.service`，執行 `p1505-duplex firmware`：

1. 透過 IPP 暫停真正印表機的佇列，避免等待中的工作搶走裝置；
2. 用 `hp-firmware` 上傳韌體，失敗最多重試 6 次；
3. 恢復佇列（無論成功與否）；
4. 把結果寫進 `STATE_DIR/firmware.json`，選單列小程式的「印表機」分頁會顯示。

查看紀錄：

```bash
journalctl -u p1505-firmware
```

## 在 Mac 上加入印表機

「系統設定 → 印表機與掃描器 → 加入印表機」，選「P1505 手動雙面 @ 伺服器名稱」，「使用」選 **Generic PostScript Printer**。不要選 HP 的驅動，也不要在 Mac 端開啟雙面，拆頁都在伺服器上做。

macOS 的 Generic PostScript 驅動送出的格式是 `application/vnd.cups-postscript`，CUPS 預設沒有把它直接轉成 PDF 的規則，會改走 `pstotiff → imagetopdf`，把整份文件壓成一頁圖片。`server/dist/local.convs` 補上用 Ghostscript 直接轉 PDF 的規則。

## 選單列小程式（macOS）

需要 macOS 27 以上。

### 編譯與安裝

```bash
macos/build.sh --install
```

用 SwiftPM 編譯、組成 `P1505 手動雙面.app`（ad-hoc 簽署），複製到「應用程式」並打開。第一次打開時，系統會詢問是否允許存取區域網路、是否允許通知，兩個都要允許。

只編譯不安裝：`macos/build.sh`，輸出在 `macos/build/`。執行測試：`cd macos && swift test`。

### 使用

- **左鍵點選單列圖示**：打開主視窗，分成三個分頁：
  - **工作**：進行中的雙面工作、正面與背面的即時狀態、「繼續列印背面」，以及「出問題了？」的補印與取消，和網頁版相同。
  - **印表機**：印表機狀態、USB 是否接上、佇列是否接受工作、最近一次韌體載入的時間與結果、正在列印與排隊中的工作（可以取消）、型號與裝置資訊；可以暫停／恢復列印佇列、列印雙面測試頁。
  - **紀錄**：最近 30 份列印工作與總張數。
- **右鍵點選單列圖示**：常用功能選單，包括等著翻面的工作（直接「繼續列印背面」）、打開各分頁、在瀏覽器打開網頁版、列印測試頁、暫停／恢復佇列、設定、結束。
- **選單列圖示**會隨狀態改變：待機、列印中、該翻面了（多份時顯示數量）、需要處理（印表機無法列印或有工作沒印完）；連不上伺服器時變成淡色。
- **通知**：該翻面時跳出通知，通知上有「繼續列印背面」按鈕，放好紙直接按就行，不用打開視窗。另外也會在雙面完成、需要補印、印表機無法列印或恢復時通知（可以在設定裡個別關掉）。

伺服器用 server-sent events 推送狀態，有變化就即時更新，不用輪詢。

### 設定

- **列印伺服器網址**：預設 `http://athlon-server.local/duplex/`。App 允許連到 `.local`、IP 位址，以及 `.lan`、`.home.arpa` 結尾的 HTTP 伺服器。
- **只顯示我送出的工作**：以 Mac 帳號名稱比對 CUPS 記錄的送件人。家裡有其他人也在用時，只會收到自己的通知。
- **通知**：四種通知可以個別開關。
- **登入時自動打開**。

### 開發：不開螢幕檢查畫面

Debug 版可以用範例資料把各分頁畫成 PNG，不需要實際點開選單列：

```bash
cd macos && swift build && .build/debug/P1505Duplex --render-previews /tmp/previews
```

## 網頁版

打開 `http://<伺服器>/duplex/`（手機也可以）。功能和選單列小程式的「工作」分頁相同，每 5 秒自動重新整理。

正面還沒印完，或 CUPS 回報印表機無法列印（佇列暫停、找不到印表機、正在載入韌體）時，「繼續」按鈕不能按，頁面上方會顯示原因。

### 卡紙、一次進兩張紙

紙張依正面出紙的順序編號：第 1 張的正面是第 1 頁、背面是第 2 頁，以此類推。打開「出問題了？」：

- **重印正面**：補印第 N 到 M 張的正面。
- **重印背面**：先把要補印背面的紙照原本方式放回紙匣，再補印第 N 到 M 張的背面。
- **取消並移除這份工作**：取消還沒印的部分。

補印會取代該面目前的工作；如果舊工作還沒印完（例如卡紙後停住），會先取消它，避免重複列印。

## API

都在 `/duplex/api/` 下，回傳 JSON；錯誤時回傳 `{"error": "…"}` 與 4xx / 5xx。

| 方法 | 路徑 | 說明 |
| --- | --- | --- |
| GET | `status` | 雙面工作與印表機狀態（每份工作含 `stage`） |
| GET | `events` | server-sent events：狀態一有變化就送出 `status` 事件，內容同上 |
| GET | `printer` | 印表機詳細資訊、韌體紀錄、佇列、最近 30 份列印紀錄 |
| GET | `version` | 伺服器版本與 API 版本 |
| POST | `jobs/{id}/continue` | 釋放保留的背面 |
| POST | `jobs/{id}/reprint` | `{"side": "front" \| "back", "first": 1, "last": 3}` |
| POST | `jobs/{id}/remove` | 取消還沒印的部分並移除 |
| POST | `queue/{job}/cancel` | 取消真正印表機佇列裡的任一工作 |
| POST | `printer/pause`、`printer/resume` | 暫停／恢復列印佇列 |
| POST | `test-page` | `{"user": "名稱"}`：以該使用者身分印一份雙面測試頁 |

`stage` 的值：`front_printing`、`front_failed`、`blocked`、`ready_to_flip`、`back_printing`、`back_failed`、`done`、`unknown`。

## 限制

P1505 不會把卡紙、缺紙、機蓋沒關等狀態回報給 CUPS：

- HP 的 CUPS 後端（`hp:`）列印時不會回報這些狀態，就算機蓋開著、工作卡在那裡，CUPS 看到的印表機狀態仍是正常。
- 印表機本身可以用 PJL（`@PJL INFO STATUS`）查詢，`hp-info` 就是這樣讀到「Door open」的。但 P1505 的 USB 連線同一時間只能給一個程式使用，列印期間裝置被 HP 後端占用，別的程式查不到；只在閒置時查詢，又抓不到最常見的「列印中途卡紙」。

所以網頁與小程式只顯示 CUPS 層級的狀態（加上 USB 是否接上）。卡紙時看印表機的指示燈，排除後用「出問題了？」補印。

狀態橫幅仍會處理 `media-jam`、`door-open` 等標準 IPP 狀態；換成會回報這些狀態的印表機時就會顯示。

## 安全性

網頁與 API 沒有登入機制，區域網路內任何人都能釋放、補印或取消工作，也能暫停佇列，和 CUPS 網頁介面預設的開放程度相當。不要把它暴露到網際網路上。

## 授權

[MIT](LICENSE)
