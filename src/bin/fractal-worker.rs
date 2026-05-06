use js_sys::{Array, Object, Reflect, Uint8ClampedArray};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};
use yew_fractal::fractal;

fn main() {
    console_error_panic_hook::set_once();

    let scope: DedicatedWorkerGlobalScope = JsValue::from(js_sys::global()).unchecked_into();
    let scope_clone = scope.clone();

    let onmessage = Closure::wrap(Box::new(move |msg: MessageEvent| {
        let data = msg.data();
        let get_f64 = |key: &str| -> f64 {
            Reflect::get(&data, &JsValue::from_str(key))
                .unwrap()
                .as_f64()
                .unwrap()
        };
        let width = get_f64("width") as usize;
        let start_y = get_f64("startY") as usize;
        let end_y = get_f64("endY") as usize;
        let zoom = get_f64("zoom");
        let offset_x = get_f64("offsetX");
        let offset_y = get_f64("offsetY");
        let max_iter = get_f64("maxIter") as u32;

        let h = end_y - start_y;
        let mut buf = vec![0u8; width * h * 4];
        for y in start_y..end_y {
            for x in 0..width {
                let c_re = (x as f64) / zoom + offset_x;
                let c_im = (y as f64) / zoom + offset_y;
                let iter = fractal::mandelbrot(c_re, c_im, max_iter);
                let idx = ((y - start_y) * width + x) * 4;
                if iter < max_iter {
                    buf[idx] = (iter * 2) as u8;
                    buf[idx + 1] = (iter * 5) as u8;
                    buf[idx + 2] = (iter * 3) as u8;
                    buf[idx + 3] = 255;
                } else {
                    buf[idx + 3] = 255;
                }
            }
        }

        let array = Uint8ClampedArray::new_with_length(buf.len() as u32);
        array.copy_from(&buf);
        let buffer = array.buffer();

        let response = Object::new();
        Reflect::set(
            &response,
            &JsValue::from_str("startY"),
            &JsValue::from(start_y as u32),
        )
        .unwrap();
        Reflect::set(&response, &JsValue::from_str("chunkData"), &buffer).unwrap();

        let transfer = Array::new();
        transfer.push(&buffer);

        scope_clone
            .post_message_with_transfer(&response.into(), &transfer.into())
            .expect("post_message failed");
    }) as Box<dyn FnMut(MessageEvent)>);

    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    // 準備完了通知（メイン側はこれを待ってから最初の描画を投げる）
    let ready = Object::new();
    Reflect::set(&ready, &JsValue::from_str("type"), &JsValue::from_str("ready")).unwrap();
    scope.post_message(&ready.into()).expect("ready post failed");
}
