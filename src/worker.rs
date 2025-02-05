use crate::fractal;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct FractalWorker {
    max_iter: u32,
}

#[wasm_bindgen]
impl FractalWorker {
    #[wasm_bindgen(constructor)]
    pub fn new(max_iter: u32) -> Self {
        Self { max_iter }
    }

    pub fn calculate_chunk(
        &self,
        width: usize,
        start_y: usize,
        end_y: usize,
        zoom: f64,
        offset_x: f64,
        offset_y: f64,
    ) -> Vec<u8> {
        let mut image_data = vec![0u8; width * (end_y - start_y) * 4];

        for y in start_y..end_y {
            for x in 0..width {
                let c_re = (x as f64) / zoom + offset_x;
                let c_im = (y as f64) / zoom + offset_y;
                let iter = fractal::mandelbrot(c_re, c_im, self.max_iter);

                let idx = ((y - start_y) * width + x) * 4;
                if iter < self.max_iter {
                    image_data[idx] = (iter * 2) as u8; // R
                    image_data[idx + 1] = (iter * 5) as u8; // G
                    image_data[idx + 2] = (iter * 3) as u8; // B
                    image_data[idx + 3] = 255; // A
                }
            }
        }
        image_data
    }
}
