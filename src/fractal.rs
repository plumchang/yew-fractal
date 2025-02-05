/// マンデルブロ集合の収束判定を行い、発散回数を返す
pub fn mandelbrot(c_re: f64, c_im: f64, max_iter: u32) -> u32 {
    // カルディオイド判定による早期リターン
    let q = (c_re - 0.25).powi(2) + c_im.powi(2);
    if q * (q + (c_re - 0.25)) <= 0.25 * c_im.powi(2) {
        return max_iter;
    }

    // 周期2の球判定
    if (c_re + 1.0).powi(2) + c_im.powi(2) <= 0.0625 {
        return max_iter;
    }

    let (mut z_re, mut z_im) = (0.0, 0.0);
    for i in 0..max_iter {
        let (z_re2, z_im2) = (z_re * z_re, z_im * z_im);
        if z_re2 + z_im2 > 4.0 {
            return i;
        }
        z_im = 2.0 * z_re * z_im + c_im;
        z_re = z_re2 - z_im2 + c_re;
    }
    max_iter
}
