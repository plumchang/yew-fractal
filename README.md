# yew-fractal

Rust + Yew + WebAssembly で実装したマンデルブロ集合のリアルタイム描画アプリです。Web Worker × 4 で並列計算しています。

姉妹プロジェクト [`react-fractal`](../react-fractal) と性能比較するためのサンプルとして作成しました。両アプリの実装と比較考察は以下の Zenn 記事にまとめています。

- 記事: [Rust(Yew) vs JavaScript(React) — マンデルブロ集合で実測した WebAssembly のリアルな速度差](https://zenn.dev/milabo/articles/rust_yew_vs_react_fractal)
- デプロイ済みデモ: https://plumchang.github.io/yew-fractal/

## 使い方

### 必要なもの

- Rust 1.70+（[rustup](https://rustup.rs/) でインストール）
- `wasm32-unknown-unknown` ターゲット
- [Trunk](https://trunkrs.dev/)（Rust → wasm のビルド/dev サーバ）

```bash
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

### 開発サーバ起動

```bash
trunk serve --release --open
```

⚠️ **必ず `--release` を付けてください**。debug ビルドの wasm は最適化が効かず、JS より大幅に遅くなります（詳細は後述）。

### 本番ビルド

```bash
trunk build --release --public-url ./
```

`dist/` 以下に成果物が出力されます。`--public-url ./` で相対パスのアセット参照になり、GitHub Pages のサブパス配信でも動作します。

### 操作方法

| 操作 | 動作 |
|---|---|
| マウスホイール | カーソル位置を中心にズームイン／アウト |
| ドラッグ | 描画領域のパン（移動） |
| 二本指タッチ（モバイル） | ピンチでズーム |
| 一本指ドラッグ（モバイル） | パン |

画面左上には現在の中心座標、ズーム倍率、フレーム計算時間（ms）、FPS を表示しています。

## アーキテクチャ概要

```
[UI スレッド (Yew)]
   ↓ postMessage({width, startY, endY, zoom, offsetX, offsetY, maxIter})
[Worker × 4 (Rust)] 各スレッドで担当チャンクを並列計算
   ↑ postMessage({startY, chunkData: ArrayBuffer})
[UI スレッド] 4 チャンクが揃ったら ImageData を合成して putImageData
```

実装の特徴：

- **Worker は使い回し**：起動時に 4 つ作ったプールを再利用
- **連続描画は間引く**：`in_flight` 中の追加リクエストは「最新の 1 件だけ」を `queued` に保留
- **ready 同期**：wasm の初期化が非同期なため、各 Worker が起動完了を `ready` メッセージで通知し、メイン側はそれを待ってから初回描画を投げる
- **transferable で受け渡し**：`post_message_with_transfer` で ArrayBuffer をゼロコピー転送

## ファイル構成

```
src/
├── lib.rs                       … 共有モジュールの公開（pub mod fractal）
├── fractal.rs                   … マンデルブロ計算本体（メイン/Worker 共通）
├── main.rs                      … メインバイナリ。Yew コンポーネント + Pool
└── bin/
    └── fractal-worker.rs        … Worker バイナリのエントリ
index.html                       … Trunk の HTML テンプレート
Cargo.toml                       … 2 つのバイナリと依存を定義
```

### `fractal.rs`

マンデルブロ集合の収束判定 `mandelbrot` 関数。React 版の `fractalWorker.ts` の `mandelbrot` 関数とロジックは完全に同じ。

性能のため早期リターンを入れています：

- メインカルディオイド判定（マンデルブロ集合の中央のハート型領域）
- 周期 2 の球判定（中心 (-1, 0)、半径 1/4 の円）

`pub` を付けて `lib.rs` 経由で公開し、メインバイナリと Worker バイナリの双方から `yew_fractal::fractal::mandelbrot` として呼び出します。

### `bin/fractal-worker.rs`

Worker のエントリポイント。`fn main()` が Worker 起動時に実行されます。

主要な処理：

1. `js_sys::global()` から `DedicatedWorkerGlobalScope` を取得
2. `Closure::wrap` で `onmessage` ハンドラを構築（Rust クロージャを JS 関数に変換）
3. `Reflect::get` で受信メッセージのプロパティを取り出し、`mandelbrot` ループを回す
4. 結果を `Uint8ClampedArray` に詰め、`post_message_with_transfer` で transferable 送信
5. 起動完了を示す `{type: "ready"}` メッセージを最後に送信

`Closure::wrap` + `forget()` パターンと、ready 通知パターンが Rust + WASM Worker の定石です。

### `main.rs`

Yew のコンポーネント `App` と、Worker プール `Pool` を実装。

#### `Pool` 構造体

`Rc<RefCell<Pool>>` の形で扱います：

- `Rc<T>`：参照カウントによる共有所有権。複数の Worker のコールバックから同じ Pool を見るために必要
- `RefCell<T>`：「不変参照しか取れない値」に対して内部可変性を提供。`borrow_mut()` で実行時に書き込み可能

これは Rust + WASM の UI コードで頻出する定型パターンです。

#### `spawn_worker` 関数

Trunk が `data-type="worker"` でビルドした Worker（`fractal-worker.js` / `fractal-worker_bg.wasm`）を、Blob 経由で起動します：

```rust
let base_uri = window().unwrap().document().unwrap().base_uri().unwrap().unwrap();
let js_url = Url::new_with_base("fractal-worker.js", &base_uri).unwrap().href();
// ... importScripts(js_url); wasm_bindgen(wasm_url); という Blob を作って Worker::new
```

`document.baseURI` を基準に絶対 URL を組み立てるので、GitHub Pages のサブパス（例：`/yew-fractal/`）配信でも動作します。

## 開発時の注意点（落とし穴）

### 落とし穴 ①: 「Worker」という名前を信用しない

過去の実装では、`FractalWorker` という名前の単なる Rust 構造体を 4 回 for ループで呼び出すコードを「並列化したつもり」になっていました。実際にはシングルスレッドで逐次計算していたため、4 並列の React 版に大きく負けていました。

修正後、Trunk の `data-type="worker"` 機能で本物の Web Worker × 4 を使うように書き換えています。本物の並列処理になっているかは、ブラウザの DevTools の Performance タブで複数 Worker スレッドが見えるかで確認できます。

### 落とし穴 ②: 必ず release ビルドで

`trunk serve` のデフォルトは debug ビルドで、wasm に LLVM 最適化や `wasm-opt` が適用されません。debug ビルドの wasm は JS より大幅に遅く、計測値が無意味になります。

このプロジェクトの `Cargo.toml` には以下が設定されています：

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

開発・計測時は必ず `trunk serve --release` で起動してください。Worker wasm のサイズが debug 174KB → release 30KB と約 1/6 になり、性能も劇的に改善します。

### 落とし穴 ③: GitHub Pages サブパス対応

`location.origin` を基準に Worker URL を組み立てると、GitHub Pages のサブパス（`https://user.github.io/yew-fractal/`）で 404 になります。

このプロジェクトでは：

- `index.html` に `<base data-trunk-public-url />` を入れて、Trunk の `--public-url` 値を `<base href>` に反映
- `spawn_worker` 関数内で `document.baseURI` を基準に `Url::new_with_base` で絶対 URL を組み立て

の対策を入れています。

## 計測指標

画面左上のオーバーレイに以下を表示：

- **Frame**: 描画リクエスト発行から `put_image_data` 完了までの時間 [ms]
- **FPS**: 直近 1 秒間に完了したフレーム数

純粋な計算性能を測るのは Frame ms です。

## クレート一覧

| クレート | 役割 |
|---|---|
| `yew` | UI フレームワーク（関数コンポーネント、フック） |
| `wasm-bindgen` | Rust ⇔ JS の境界。`JsValue`, `Closure`, `JsCast` 等 |
| `js-sys` | JS 標準オブジェクト（`Array`, `Object`, `Reflect`, `Uint8ClampedArray` 等） |
| `web-sys` | ブラウザ Web API（`Worker`, `MessageEvent`, `HtmlCanvasElement` 等） |
| `console_error_panic_hook` | パニックをブラウザコンソールに見やすく出すフック |

## 関連リンク

- 姉妹プロジェクト: [`react-fractal`](../react-fractal)（React + TypeScript 版）
- マンデルブロ集合: [Wikipedia](https://ja.wikipedia.org/wiki/マンデルブロ集合)
- Trunk 公式: [trunkrs.dev](https://trunkrs.dev/)
- Trunk Web Worker サンプル: [trunk-rs/trunk examples/webworker](https://github.com/trunk-rs/trunk/tree/main/examples/webworker)
- Yew 公式: [yew.rs](https://yew.rs/)
- wasm-bindgen Book: [rustwasm.github.io/wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/)
