/// マンデルブロ集合の収束判定を行い、発散までの反復回数を返す。
///
/// `pub` を付けることでクレート外（main バイナリと worker バイナリの双方）から呼び出せる。
/// このファイルは `src/lib.rs` で `pub mod fractal;` と公開されているので、
/// `yew_fractal::fractal::mandelbrot` として両方の bin から参照される。
///
/// 引数の型に注目:
///  - `f64`: JS の number と同じ 64bit 浮動小数点。Rust では型を明示する必要がある
///  - `u32`: 符号なし 32bit 整数。JS の number（double）と違い、整数として扱われる
///
/// 戻り値の型 `u32` はそのまま返り値の型注釈。Rust では関数末尾の式（セミコロン無し）が戻り値になる。
pub fn mandelbrot(c_re: f64, c_im: f64, max_iter: u32) -> u32 {
    // ===== 早期リターン: メインカルディオイド =====
    // JS 版の `Math.pow(c_re - 0.25, 2)` に相当するのが `(c_re - 0.25).powi(2)`。
    // `powi` は整数乗の専用メソッドで、内部的には乗算を展開してくれる。
    // ただしホットパスでは `let dx = c_re - 0.25; dx * dx` の方が高速な場合もある（最適化のヒント）。
    let q = (c_re - 0.25).powi(2) + c_im.powi(2);
    if q * (q + (c_re - 0.25)) <= 0.25 * c_im.powi(2) {
        return max_iter;
    }

    // ===== 早期リターン: 周期2の球（半径 1/4 の円） =====
    // (c_re + 1)² + c_im² <= 1/16 = 0.0625
    if (c_re + 1.0).powi(2) + c_im.powi(2) <= 0.0625 {
        return max_iter;
    }

    // ===== 通常の漸化式ループ =====
    // タプルパターンによる多重代入。JS だと
    //   let z_re = 0, z_im = 0;
    // と書く部分。`mut` を付けないと再代入できない（Rust のデフォルト不変性）。
    let (mut z_re, mut z_im) = (0.0, 0.0);

    // `0..max_iter` は半開区間のイテレータ。Rust の for は常にイテレータを舐める形。
    // `i` は発散時にそのまま戻り値として使う。
    for i in 0..max_iter {
        // 一時変数も同じくタプルで分解代入
        let (z_re2, z_im2) = (z_re * z_re, z_im * z_im);

        // |z|² > 4 で発散確定 → 現在の反復回数 `i` を返す
        if z_re2 + z_im2 > 4.0 {
            return i;
        }

        // 漸化式を進める。JS 版と完全に同じ計算。
        // 順序重要: z_im を更新する前に z_re² と z_im² を計算済み（上のタプルで）なので問題ない
        z_im = 2.0 * z_re * z_im + c_im;
        z_re = z_re2 - z_im2 + c_re;
    }

    // ループを抜けた = 発散しなかった = 集合の点とみなす。
    // 末尾にセミコロンが無いことに注目: これが「この値を返す」意味になる（return キーワード不要）。
    max_iter
}

/// 4 点を同時に計算する SIMD 版（f32x4）。
///
/// 各レーンが独立した複素数 c = (c_re[i], c_im[i]) を持ち、WASM SIMD の v128 上で
/// 4 並列に漸化式 z = z² + c を進める。理論ピークはスカラ版の 4 倍。
///
/// f32 を選んだ理由:
///  - WASM SIMD の f64 はレーン 2 個（最大 2 倍）に対し f32 は 4 個取れる
///  - マンデルブロの表示精度は通常 6〜7 桁あれば十分（深いズームを除く）
///
/// 注意:
///  - レーン毎に発散タイミングが違うため、全レーン発散するまでループを止められない。
///    発散済みレーンの結果は最初に発散した時の i を保持し、それ以降は更新しない。
///  - 早期 exit (cardioid / bulb) もレーン毎にマスクで扱う。
pub fn mandelbrot_x4(c_re: [f32; 4], c_im: [f32; 4], max_iter: u32) -> [u32; 4] {
    // .cargo/config.toml で +simd128 を全体有効化しているので、
    // この関数は wasm32 ターゲットでのみ意味のある実装を提供する。
    #[cfg(target_arch = "wasm32")]
    unsafe {
        mandelbrot_x4_simd(c_re, c_im, max_iter)
    }

    // 非 wasm ターゲット（テスト・解析等）向けのスカラフォールバック。
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut out = [0u32; 4];
        for i in 0..4 {
            out[i] = mandelbrot(c_re[i] as f64, c_im[i] as f64, max_iter);
        }
        out
    }
}

/// WASM SIMD intrinsics による本体。
///
/// `#[target_feature(enable = "simd128")]` を付けるため安全境界として `unsafe fn`。
/// 呼び出し側はビルド時に simd128 が有効なことを保証する責任を負う。
#[cfg(target_arch = "wasm32")]
#[target_feature(enable = "simd128")]
unsafe fn mandelbrot_x4_simd(c_re: [f32; 4], c_im: [f32; 4], max_iter: u32) -> [u32; 4] {
    use std::arch::wasm32::*;

    // 配列からレーンを構築。`f32x4(a, b, c, d)` はアライメント要件なしで v128 を作れる。
    let c_re_v = f32x4(c_re[0], c_re[1], c_re[2], c_re[3]);
    let c_im_v = f32x4(c_im[0], c_im[1], c_im[2], c_im[3]);

    // ===== 早期判定: メインカルディオイド =====
    //   q = (c_re - 0.25)² + c_im²
    //   q * (q + (c_re - 0.25)) <= 0.25 * c_im²  ならカルディオイド内
    let cre_q = f32x4_sub(c_re_v, f32x4_splat(0.25));
    let im2 = f32x4_mul(c_im_v, c_im_v);
    let q = f32x4_add(f32x4_mul(cre_q, cre_q), im2);
    let card_lhs = f32x4_mul(q, f32x4_add(q, cre_q));
    let card_rhs = f32x4_mul(f32x4_splat(0.25), im2);
    // 比較結果は「真ならレーン全 1 ビット、偽なら全 0」のビットマスク
    let in_cardioid = f32x4_le(card_lhs, card_rhs);

    // ===== 早期判定: 周期 2 の球 =====
    //   (c_re + 1)² + c_im² <= 1/16
    let cre_p1 = f32x4_add(c_re_v, f32x4_splat(1.0));
    let bulb = f32x4_add(f32x4_mul(cre_p1, cre_p1), im2);
    let in_bulb = f32x4_le(bulb, f32x4_splat(0.0625));

    // どちらかに当てはまるレーンは「集合内」として確定（戻り値 = max_iter）
    let in_set = v128_or(in_cardioid, in_bulb);

    // result: 各レーンの戻り値候補。初期値 max_iter（集合内とみなす）。
    let mut result = u32x4_splat(max_iter);
    // done: 既に確定したレーン（発散済 or 集合内）のマスク。1 のレーンはこれ以上更新しない。
    let mut done = in_set;

    let mut z_re = f32x4_splat(0.0);
    let mut z_im = f32x4_splat(0.0);
    let four = f32x4_splat(4.0);
    let two = f32x4_splat(2.0);

    for i in 0..max_iter {
        let z_re2 = f32x4_mul(z_re, z_re);
        let z_im2 = f32x4_mul(z_im, z_im);
        let mag2 = f32x4_add(z_re2, z_im2);
        // 発散レーン: |z|² > 4
        let diverged = f32x4_gt(mag2, four);
        // 「今回初めて発散したレーン」だけに i を書き込みたい:
        //   new_div = diverged AND NOT done
        let new_div = v128_and(diverged, v128_not(done));
        // bitselect(t, f, mask): 各ビットで mask=1 なら t を、0 なら f を採用。
        // 新規発散レーンだけ i に更新、それ以外は元の result を維持。
        result = v128_bitselect(u32x4_splat(i), result, new_div);
        // 発散したレーンを done に取り込む
        done = v128_or(done, diverged);

        // 全レーン確定したら早期打ち切り（i32x4 各レーンの最上位ビットを 4bit に集約）
        if i32x4_bitmask(done) == 0b1111 {
            break;
        }

        // 漸化式更新（スカラ版と同じ計算を 4 並列で）
        z_im = f32x4_add(f32x4_mul(two, f32x4_mul(z_re, z_im)), c_im_v);
        z_re = f32x4_add(f32x4_sub(z_re2, z_im2), c_re_v);
    }

    [
        u32x4_extract_lane::<0>(result),
        u32x4_extract_lane::<1>(result),
        u32x4_extract_lane::<2>(result),
        u32x4_extract_lane::<3>(result),
    ]
}
