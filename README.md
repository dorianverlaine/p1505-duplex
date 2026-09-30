# p1505-duplex-rs

讓接在 Linux CUPS 伺服器上的 **HP LaserJet P1505** 也能雙面列印：正面先印，把紙放回紙匣，在網頁上按「繼續」，再印背面。卡紙或一次進了兩張紙時，可以只補印出問題的那幾張。

整個工具是一支用 Rust 寫的靜態執行檔，同時也負責在印表機開機時自動載入韌體。

## 為什麼需要這個

P1505 **沒有雙面單元**。Windows / macOS 上 HP 原廠驅動的「雙面列印」其實是在電腦端完成的：先送奇數頁，跳出視窗請你把紙翻面放回紙匣，按下繼續後再送偶數頁。

印表機接在 CUPS 伺服器上時，電腦只是把整份檔案交給伺服器，沒有人能在你的電腦上跳出視窗，這個功能就不見了。這個專案在伺服器上把同樣的流程做回來，用一個網頁取代那個視窗。

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
http://<伺服器>/duplex/
  顯示正面、背面與印表機的即時狀態
  按「繼續列印背面」→ 釋放保留的工作
  「出問題了？」→ 補印第 N–M 張的正面或背面
```

一支執行檔依呼叫方式扮演三種角色：

| 角色 | 呼叫方式 | 以誰的身分執行 |
| --- | --- | --- |
| CUPS 後端 | CUPS 執行 `/usr/lib/cups/backend/duplex` | `lp` |
| 網頁 | `p1505-duplex web`（systemd） | `p1505-duplex`（`lpadmin` 群組） |
| 韌體載入 | `p1505-duplex firmware`（udev → systemd） | `root` |

### 和 CUPS 溝通：IPP over domain socket

所有 CUPS 操作都直接透過 IPP，經由 `/run/cups/cups.sock` 進行，不呼叫 `lp`、`lpstat` 等指令、也不解析它們的輸出：

- 在 domain socket 上，cupsd 知道對方的 uid。管理操作（釋放、取消別人的工作、暫停佇列）帶上 `Authorization: PeerCred <使用者>`，只要該使用者屬於 CUPS 的 SystemGroup（`lpadmin`）就會通過，不需要密碼或憑證。
- 送出列印工作時不帶認證，工作的擁有者就是原本送件的使用者，而不是後端執行時的 `lp`。
- 工作狀態（排隊、保留、列印中、已停止、失敗、完成）和印表機狀態（卡紙、缺紙、機蓋未關、離線…）都直接讀 IPP 屬性，不受系統語系影響。

### PDF 處理

用 [lopdf](https://crates.io/crates/lopdf) 挑選頁面、重新排序、設定 `/Rotate`、產生空白頁。後端收到的 PDF 一定是 CUPS 的 `pdftopdf` 輸出，格式規整，執行時不需要 qpdf 或 Ghostscript。

## 需求

- Debian / Ubuntu，CUPS 與 HPLIP，P1505 已能用 `hp:/usb/...` 單面列印
- HPLIP 專有外掛（`hp-plugin`），提供 `hp-firmware` 與 P1505 韌體檔
- `cups-filters` 的 `Generic-PDF_Printer-PDF.ppd`
- nginx（或任何能反向代理的網頁伺服器）
- 編譯：Docker（在 macOS 上可用 OrbStack），或本機 Rust 工具鏈

## 編譯

```bash
./build.sh
```

在 Docker 的 `rust:alpine`（linux/amd64）裡編譯出靜態連結的 x86_64 執行檔：`target/linux/x86_64-unknown-linux-musl/release/p1505-duplex`。

執行測試：

```bash
cargo test
```

## 安裝

把執行檔、`dist/` 和 `install.sh` 複製到伺服器同一個目錄，以 root 執行：

```bash
sudo ./install.sh
```

`install.sh` 會：

- 把執行檔裝到 `/usr/local/bin/p1505-duplex`，並複製一份到 `/usr/lib/cups/backend/duplex`（Ubuntu 的 AppArmor 只允許 cupsd 執行後端目錄裡的檔案，所以不能用 symlink）
- 安裝 `/etc/cups/local.convs`、systemd 服務和 udev 規則
- 建立系統帳號 `p1505-duplex`（`lpadmin` 群組）與 `/var/spool/p1505-duplex`
- 建立共享佇列 `P1505_Duplex`，顯示名稱「P1505 手動雙面」
- 已存在的 `/etc/p1505-duplex.conf` 不會被覆蓋

最後參考 `dist/nginx.conf`，把 `/duplex/` 反向代理到 `127.0.0.1:8631`。

## 設定

`/etc/p1505-duplex.conf`（範例見 `dist/p1505-duplex.conf`）：

| 鍵 | 預設值 | 說明 |
| --- | --- | --- |
| `PRINTER` | `HP_LaserJet_P1505` | 真正印表機的 CUPS 佇列名稱 |
| `EVEN_ORDER` | `reverse` | 背面的送出順序：`reverse` 或 `forward` |
| `EVEN_ROTATE` | `180` | 背面額外旋轉：`0` 或 `180` |
| `LISTEN` | `127.0.0.1:8631` | 網頁監聽位址 |
| `STATE_DIR` | `/var/spool/p1505-duplex` | 工作紀錄與原始 PDF |

改完立即生效（網頁需要 `systemctl restart p1505-duplex-web`）。每份工作會記下送出當時的順序與旋轉設定，補印時沿用，不受之後修改影響。

### 校正背面方向

背面的順序和方向取決於紙張路徑，以及你怎麼把紙疊放回紙匣。第一次使用時印一份測試頁：

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

## 在 Mac 上加入印表機

「系統設定 → 印表機與掃描器 → 加入印表機」，選「P1505 手動雙面 @ 伺服器名稱」，「使用」選 **Generic PostScript Printer**。不要選 HP 的驅動，也不要在 Mac 端開啟雙面，拆頁都在伺服器上做。

macOS 的 Generic PostScript 驅動送出的格式是 `application/vnd.cups-postscript`，CUPS 預設沒有把它直接轉成 PDF 的規則，會改走 `pstotiff → imagetopdf`，把整份文件壓成一頁圖片。`dist/local.convs` 補上用 Ghostscript 直接轉 PDF 的規則。

## 使用

1. 列印到「P1505 手動雙面」，正面會馬上印出。
2. 正面印完後，把整疊紙放回紙匣。
3. 打開 `http://<伺服器>/duplex/`（手機也可以），按「繼續列印背面」。

正面還沒印完、或印表機有狀況（卡紙、缺紙、機蓋沒關、離線）時，「繼續」按鈕不能按，頁面上方會顯示印表機狀態。

### 卡紙、一次進兩張紙

紙張依正面出紙的順序編號：第 1 張的正面是第 1 頁、背面是第 2 頁，以此類推。打開卡片下方的「出問題了？」：

- **重印正面**：補印第 N 到 M 張的正面。
- **重印背面**：先把要補印背面的紙照原本方式放回紙匣，再補印第 N 到 M 張的背面。
- **取消並移除這份工作**：取消還沒印的部分。

補印會取代該面目前的工作；如果舊工作還沒印完（例如卡紙後停住），會先取消它，避免重複列印。

## 韌體自動載入

插上印表機或開機時，udev 啟動 `p1505-firmware.service`，執行 `p1505-duplex firmware`：

1. 透過 IPP 暫停真正印表機的佇列，避免等待中的工作搶走裝置；
2. 用 `hp-firmware` 上傳韌體，失敗最多重試 6 次；
3. 恢復佇列（無論成功與否）。

查看紀錄：

```bash
journalctl -u p1505-firmware
```

## 安全性

網頁沒有登入機制，區域網路內任何人都能釋放、補印或取消工作，和 CUPS 網頁介面預設的開放程度相當。不要把它暴露到網際網路上。

## 授權

[MIT](LICENSE)
