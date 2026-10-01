# sen-turbo-fieldfare

Runtime LLM [TurboFieldfare](https://github.com/drumih/turbo-fieldfare) cho
[SenClaw](https://github.com/SenClaw/senclaw): chạy Gemma 4 26B-A4B trên Apple
Silicon với khoảng 2 GB RAM, phục vụ API tương thích OpenAI. Daemon khởi chạy
process này và nói chuyện qua loopback — cùng cách `sen-mlx` chạy MLX — nên
daemon không link Metal.

Process này bọc binary `TurboFieldfareServer` đã build sẵn. `/health` trả 503
khi đang nạp trọng số, mọi route khác yêu cầu bearer token, và
`POST /runtime/shutdown` tắt process.

Hợp đồng: [`senclaw/docs/runtime-protocol.md`](../senclaw/docs/runtime-protocol.md) §4.2.

## Không cần Swift để chạy

`TurboFieldfareServer` là file thực thi Mach-O. Nó link runtime Swift có sẵn
trong macOS (`/usr/lib/swift`), giống app link Foundation. Máy không cài
`swift`, Xcode, hay Swift toolchain vẫn chạy được.

`make package`, `make install-local`, và `make run-dev` không gọi `swift`.
Chúng dùng binary đã có ở `turbo-fieldfare-senclaw/.build/release/`, hoặc
binary trong gói `dist/`. `make engine` mới biên dịch lại từ source, và chỉ
khi nào bạn muốn build engine.

## Yêu cầu

- Mac Apple Silicon (darwin-arm64), macOS 26, Metal 4
- Một thư mục `.gturbo` đã cài xong (khoảng 14.3 GB). Bản trên máy này đã được
  nối vào `~/.senclaw/local-models/gemma4.gturbo`.

## Cài từ bản release trên GitHub

Settings → Runtime → TurboFieldfare → Cài. Daemon tải
`sen-turbo-fieldfare-<version>-darwin-arm64.tar.gz` từ release `v<version>`
và đối chiếu file `.sha256` đăng kèm.

Không mở app thì:

```bash
make install-release
```

Lệnh này không gọi `swift` và không build Rust. Nó tải đúng archive của tag
`v` + version trong `Cargo.toml`.

## Cài runtime từ source

```bash
make package          # không gọi swift
make install-local
```

## Cài model

Nếu chưa có `.gturbo`, dùng binary repack đi kèm gói (cũng không cần `swift`):

```bash
dist/sen-turbo-fieldfare-0.1.0-darwin-arm64/bin/TurboFieldfareRepack \
  --output ~/.senclaw/local-models/gemma4.gturbo \
  --overwrite
```

Ảnh (tuỳ chọn, khoảng 1.1 GB, cần chip M2 trở lên):

```bash
dist/sen-turbo-fieldfare-0.1.0-darwin-arm64/bin/TurboFieldfareRepack \
  --vision-output ~/.senclaw/local-models/gemma4.vision.gturbo \
  --text-model ~/.senclaw/local-models/gemma4.gturbo
```

Runtime từ chối thư mục không phải đúng checkpoint này: `modelID` phải là
`mlx-community/gemma-4-26b-a4b-it-4bit` và `sourceSnapshotHash` phải là
`sha256:bf198c9f5ea6462addca1966e5dd669c407537a876e82cf06db9084c5c850b13`
(revision `0d77464eeb233a2da68ebf9d7dc4edaac7db956d`). Daemon cũng chỉ liệt kê
đúng bản đó. Nút tải trong Settings gọi `TurboFieldfareRepack` của gói đã cài,
không tải snapshot MLX thô.

Thư mục `*.vision.gturbo` là gói ảnh đi kèm, không hiện thành một model riêng.

## Chạy

Trong Settings → Runtime, chọn **sen-turbo-fieldfare** cho slot
**TurboFieldfare**. Model hiện trong bộ chọn như các model local khác
(`local:<key>`).

Chạy tay, không token, không watchdog:

```bash
make run-dev MODEL=~/.senclaw/local-models/gemma4.gturbo
```

`GET /health` trả 503 đến khi engine nạp xong, rồi 200.
`POST /v1/chat/completions` dùng đúng `model` id mà `GET /v1/models` trả về
(khi daemon khởi chạy, đó là model key).

Chỉ một process TurboFieldfare được giữ model tại một thời điểm. Đóng app
TurboFieldfareMac, CLI, và server độc lập trước khi SenClaw nạp model.
