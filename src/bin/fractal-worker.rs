// ===== クレートの役割整理 =====
// js_sys:        JS 標準オブジェクト（Array, Object, Reflect 等）への Rust バインディング
// wasm_bindgen:  Rust と JS の境界を取り持つ基盤（JsValue, Closure, JsCast 等）
// web_sys:       ブラウザ Web API（Worker, MessageEvent, Canvas 等）への Rust バインディング
// yew_fractal:   自分のクレートの lib.rs。fractal モジュールを共有するために import

use js_sys::{Array, Object, Reflect, Uint8ClampedArray};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};
use yew_fractal::fractal;

// この `main` 関数が Worker のエントリポイント。
// Trunk が `data-type="worker"` でビルドした wasm は、Worker 起動時に
// `wasm_bindgen` の初期化を経てこの main が呼ばれる仕組み。
fn main() {
    // Rust のパニック（unwrap 失敗、配列範囲外、等）が起きた時に、
    // ブラウザのコンソールにわかりやすいスタックトレースを出すためのフック。
    // これがないと「unreachable executed」のような暗号めいたエラーしか出ない。
    console_error_panic_hook::set_once();

    // Worker の中では `self` が DedicatedWorkerGlobalScope（onmessage や postMessage を持つ）。
    // js_sys::global() は JS の globalThis 相当の JsValue を返すので、
    // それを DedicatedWorkerGlobalScope 型へキャストする。
    //
    // unchecked_into は「実行時の型チェック無しで強制キャスト」する操作。
    // ここでは Worker 内で動くことが確実なので unchecked でよい（unsafe 寄りの選択）。
    let scope: DedicatedWorkerGlobalScope = JsValue::from(js_sys::global()).unchecked_into();
    // クロージャ内で post_message する用に複製。`Worker` 系は内部で Rc 相当のため `clone` は安価。
    let scope_clone = scope.clone();

    // ===== onmessage ハンドラの構築 =====
    // JS の `self.onmessage = (e) => {...}` を Rust で書くには、
    // Rust のクロージャを Closure::wrap で JS 関数に変換する必要がある。
    //
    // Box<dyn FnMut(MessageEvent)> としているのは、JS 側がこのクロージャを「型消去された
    // 関数オブジェクト」として保持するため。FnMut なので内部状態を変更できる。
    let onmessage = Closure::wrap(Box::new(move |msg: MessageEvent| {
        // メッセージのペイロード（postMessage の第1引数）は JsValue として取れる。
        // メイン側からは { width, startY, endY, zoom, offsetX, offsetY, maxIter } という
        // プレーン JS オブジェクトを送っている。
        let data = msg.data();

        // JS オブジェクトのプロパティを安全に取り出すヘルパ。
        // Reflect::get は Result<JsValue, JsValue> を返す（プロパティ取得は失敗しうる）。
        // それを f64 として扱うため as_f64() で Option<f64> にし、unwrap で取り出している。
        // 簡略化のため unwrap だらけだが、本番コードならエラー処理を入れる場所。
        let get_f64 = |key: &str| -> f64 {
            Reflect::get(&data, &JsValue::from_str(key))
                .unwrap()
                .as_f64()
                .unwrap()
        };

        // JS 側の値はすべて f64（number）として届くので、
        // 必要に応じて usize や u32 にキャストする。`as` は明示的な型変換キーワード。
        let width = get_f64("width") as usize;
        let start_y = get_f64("startY") as usize;
        let end_y = get_f64("endY") as usize;
        let zoom = get_f64("zoom");
        let offset_x = get_f64("offsetX");
        let offset_y = get_f64("offsetY");
        let max_iter = get_f64("maxIter") as u32;

        // ===== 計算本体 =====
        // React 版の fractalWorker.ts と同じロジックを Rust で書いている。
        // `vec![0u8; ...]` は「0 で初期化された Vec<u8> をその長さ分確保」する省略記法。
        let h = end_y - start_y;
        let mut buf = vec![0u8; width * h * 4];

        // ピクセル色塗りクロージャ。
        // SIMD ループとスカラ端数ループの両方から呼ぶため抽出している。
        //
        // ⚠️ React 版との挙動の違いに注意:
        //   - Rust の `as u8` キャストは「256 で割った余り」（モジュロ）を取る。
        //     例: iter=52, iter*5=260 → 260 % 256 = 4
        //   - JS の Uint8ClampedArray は範囲外を 255 にクランプする。
        //     例: iter*5=260 → 255
        //
        // つまりこのコードは React 版と「同じ計算」のつもりで書かれているが、
        // 実際には iter が大きい領域（深ズーム時の境界部）で全く違う色になる。
        // 結果として Rust 版では紫や黄色が混ざる派手な配色になる。
        //
        // 一致させたい場合は以下のようにクランプを明示する:
        //   buf[idx + 1] = (iter * 5).min(255) as u8;
        //
        // 今回はあえて修正せず、整数オーバーフローの扱いの違いという学びの題材
        // として残している（記事のおまけネタとしても触れている）。
        //
        // なお、後続で Uint8ClampedArray::copy_from で詰め直しているが、
        // 「u8 として 0〜255 の範囲内に丸めた値」をコピーするだけなので
        // クランプは効かない（クランプは「256 を超える値を代入する」時のみ発動）。
        let put_pixel = |buf: &mut [u8], idx: usize, iter: u32| {
            if iter < max_iter {
                // 発散：色のグラデーション
                buf[idx] = (iter * 2) as u8;
                buf[idx + 1] = (iter * 5) as u8;
                buf[idx + 2] = (iter * 3) as u8;
                buf[idx + 3] = 255;
            } else {
                // 発散しなかった = 集合に含まれる：黒
                // R, G, B は vec! で 0 初期化済みなので alpha だけ設定すればよい
                buf[idx + 3] = 255;
            }
        };

        // x 方向 4 ピクセル単位で SIMD（f32x4）処理し、端数はスカラで埋める。
        // width が 4 の倍数でないケースに備え、x4_end までを SIMD、以降をスカラ。
        let x4_end = width - (width % 4);

        for y in start_y..end_y {
            // 同一行内では c_im は不変。f32 と f64 を両方持っておく
            // （SIMD 版は f32、端数のスカラ版は元と同じ f64 精度を維持）。
            let c_im_f64 = (y as f64) / zoom + offset_y;
            let c_im_f32 = c_im_f64 as f32;
            let row_base = (y - start_y) * width;

            // ===== SIMD ループ：4 点同時計算 =====
            let mut x = 0;
            while x < x4_end {
                let c_re_arr = [
                    ((x as f64) / zoom + offset_x) as f32,
                    (((x + 1) as f64) / zoom + offset_x) as f32,
                    (((x + 2) as f64) / zoom + offset_x) as f32,
                    (((x + 3) as f64) / zoom + offset_x) as f32,
                ];
                let c_im_arr = [c_im_f32; 4];
                let iters = fractal::mandelbrot_x4(c_re_arr, c_im_arr, max_iter);

                for k in 0..4 {
                    let idx = (row_base + x + k) * 4;
                    put_pixel(&mut buf, idx, iters[k]);
                }
                x += 4;
            }

            // ===== 端数：従来通り 1 点ずつスカラ計算 =====
            while x < width {
                let c_re = (x as f64) / zoom + offset_x;
                let iter = fractal::mandelbrot(c_re, c_im_f64, max_iter);
                let idx = (row_base + x) * 4;
                put_pixel(&mut buf, idx, iter);
                x += 1;
            }
        }

        // ===== 結果を JS 側に送る =====
        // wasm のリニアメモリにある Vec<u8> を、JS 側で扱える Uint8ClampedArray にコピー。
        // ここで wasm ↔ JS のメモリ境界を越えるためのデータコピーが発生する（避けられない）。
        let array = Uint8ClampedArray::new_with_length(buf.len() as u32);
        array.copy_from(&buf);

        // 内部の ArrayBuffer を取り出す（transferable で渡すため）
        let buffer = array.buffer();

        // 返信メッセージを { startY, chunkData } の形のオブジェクトとして組み立てる。
        // Rust の構造体ではなく JS のプレーンオブジェクトを動的に作っている。
        let response = Object::new();
        Reflect::set(
            &response,
            &JsValue::from_str("startY"),
            &JsValue::from(start_y as u32),
        )
        .unwrap();
        Reflect::set(&response, &JsValue::from_str("chunkData"), &buffer).unwrap();

        // transferable リスト。ここに含めた ArrayBuffer は「所有権移譲」され、
        // Worker 側からは使えなくなる代わりにコピーが発生しない（zero-copy 受け渡し）。
        let transfer = Array::new();
        transfer.push(&buffer);

        scope_clone
            .post_message_with_transfer(&response.into(), &transfer.into())
            .expect("post_message failed");
    }) as Box<dyn FnMut(MessageEvent)>);

    // 構築したクロージャを Worker のグローバルスコープの onmessage に登録。
    // `as_ref().unchecked_ref()` で Closure → &Function の参照を取り出す。
    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

    // ★ 重要: forget で Rust 側の所有権を明示的に放棄する。
    //   onmessage を Drop してしまうと JS 側に登録した関数が無効になる。
    //   Worker は終了するまで生き続けるので、メモリリークは実質問題にならない。
    onmessage.forget();

    // ===== 準備完了通知 =====
    // wasm のロードと初期化は非同期で起こるため、Worker 起動直後に
    // メイン側が postMessage を投げると onmessage が登録される前に到着し、捨てられてしまう。
    // そこで「Worker 側で onmessage の登録が完了した」ことをメインに伝えるために
    // ready メッセージを送り、メイン側は ready を受信してから最初の描画を投げる。
    let ready = Object::new();
    Reflect::set(&ready, &JsValue::from_str("type"), &JsValue::from_str("ready")).unwrap();
    scope.post_message(&ready.into()).expect("ready post failed");
}
