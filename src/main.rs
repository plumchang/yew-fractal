use wasm_bindgen::prelude::*;
use web_sys::{
    window, CanvasRenderingContext2d, HtmlCanvasElement, MouseEvent, TouchEvent, WheelEvent,
};
use yew::prelude::*;
mod fractal;
mod worker;

#[function_component]
fn App() -> Html {
    let canvas_ref = use_node_ref();
    let zoom = use_state(|| 200.0);
    let offset_x = use_state(|| -2.0);
    let offset_y = use_state(|| -1.0);
    let is_dragging = use_state(|| false);
    let last_mouse_pos = use_state(|| (0.0, 0.0));
    let last_touch_pos = use_state(|| (0.0, 0.0));
    let last_pinch_distance = use_state(|| None::<f64>);

    let draw_fractal = {
        let canvas_ref = canvas_ref.clone();
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();

        Callback::from(move |_| {
            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                let context = canvas
                    .get_context("2d")
                    .unwrap()
                    .unwrap()
                    .dyn_into::<CanvasRenderingContext2d>()
                    .unwrap();

                let width = canvas.width() as usize;
                let height = canvas.height() as usize;

                // Workerを4つ作成して並列処理
                let worker = worker::FractalWorker::new(100);
                let chunk_size = height / 4;

                let mut image_data = vec![0u8; width * height * 4];

                for i in 0..4 {
                    let start_y = i * chunk_size;
                    let end_y = if i == 3 { height } else { (i + 1) * chunk_size };

                    let chunk_data =
                        worker.calculate_chunk(width, start_y, end_y, *zoom, *offset_x, *offset_y);

                    // チャンクデータをimage_dataにコピー
                    let start_idx = start_y * width * 4;
                    let end_idx = end_y * width * 4;
                    image_data[start_idx..end_idx].copy_from_slice(&chunk_data);
                }

                let data = web_sys::ImageData::new_with_u8_clamped_array(
                    wasm_bindgen::Clamped(&image_data),
                    width as u32,
                )
                .unwrap();

                context.put_image_data(&data, 0.0, 0.0).unwrap();
            }
        })
    };

    let on_mouse_down: Callback<MouseEvent> = {
        let canvas_ref = canvas_ref.clone();
        let last_mouse_pos = last_mouse_pos.clone();
        let is_dragging = is_dragging.clone();

        Callback::from(move |event: MouseEvent| {
            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                let x = event.client_x() as f64 - canvas.client_left() as f64;
                let y = event.client_y() as f64 - canvas.client_top() as f64;
                is_dragging.set(true);
                last_mouse_pos.set((x, y));
                event.prevent_default();
            }
        })
    };

    let on_mouse_move = {
        let canvas_ref = canvas_ref.clone();
        let last_mouse_pos = last_mouse_pos.clone();
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let is_dragging = is_dragging.clone();
        let draw_fractal = draw_fractal.clone();

        Callback::from(move |event: MouseEvent| {
            if *is_dragging {
                if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                    let x = event.client_x() as f64 - canvas.client_left() as f64;
                    let y = event.client_y() as f64 - canvas.client_top() as f64;
                    let (last_x, last_y) = *last_mouse_pos;

                    offset_x.set(*offset_x - (x - last_x) / *zoom);
                    offset_y.set(*offset_y - (y - last_y) / *zoom);
                    last_mouse_pos.set((x, y));
                    draw_fractal.emit(());
                }
            }
        })
    };

    let on_mouse_up = {
        let is_dragging = is_dragging.clone();
        Callback::from(move |_| {
            is_dragging.set(false);
        })
    };

    let on_wheel = {
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let draw_fractal = draw_fractal.clone();
        let canvas_ref = canvas_ref.clone();

        Callback::from(move |event: WheelEvent| {
            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                let mouse_x = event.client_x() as f64 - canvas.client_left() as f64;
                let mouse_y = event.client_y() as f64 - canvas.client_top() as f64;

                let factor = if event.delta_y() > 0.0 { 1.1 } else { 0.9 };
                let new_zoom = *zoom * factor;

                let dx = mouse_x / *zoom - mouse_x / new_zoom;
                let dy = mouse_y / *zoom - mouse_y / new_zoom;

                offset_x.set(*offset_x + dx);
                offset_y.set(*offset_y + dy);
                zoom.set(new_zoom);

                draw_fractal.emit(());
                event.prevent_default();
            }
        })
    };

    let on_touch_start = {
        let canvas_ref = canvas_ref.clone();
        let last_touch_pos = last_touch_pos.clone();
        let last_pinch_distance = last_pinch_distance.clone();
        let is_dragging = is_dragging.clone();

        Callback::from(move |event: TouchEvent| {
            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                is_dragging.set(true);

                let touches = event.touches();
                if touches.length() == 1 {
                    // シングルタッチの場合
                    let touch = touches.get(0).unwrap();
                    let x = touch.client_x() as f64 - canvas.client_left() as f64;
                    let y = touch.client_y() as f64 - canvas.client_top() as f64;
                    last_touch_pos.set((x, y));
                    last_pinch_distance.set(None);
                } else if touches.length() == 2 {
                    // ピンチ操作の場合
                    let touch1 = touches.get(0).unwrap();
                    let touch2 = touches.get(1).unwrap();
                    let dx = touch1.client_x() - touch2.client_x();
                    let dy = touch1.client_y() - touch2.client_y();
                    let distance = ((dx * dx + dy * dy) as f64).sqrt();
                    last_pinch_distance.set(Some(distance));
                }
            }
        })
    };

    let on_touch_move = {
        let canvas_ref = canvas_ref.clone();
        let last_touch_pos = last_touch_pos.clone();
        let last_pinch_distance = last_pinch_distance.clone();
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let is_dragging = is_dragging.clone();
        let draw_fractal = draw_fractal.clone();

        Callback::from(move |event: TouchEvent| {
            if *is_dragging {
                if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                    let touches = event.touches();

                    if touches.length() == 1 {
                        // シングルタッチの移動
                        let touch = touches.get(0).unwrap();
                        let x = touch.client_x() as f64 - canvas.client_left() as f64;
                        let y = touch.client_y() as f64 - canvas.client_top() as f64;
                        let (last_x, last_y) = *last_touch_pos;

                        offset_x.set(*offset_x - (x - last_x) / *zoom);
                        offset_y.set(*offset_y - (y - last_y) / *zoom);
                        last_touch_pos.set((x, y));
                        draw_fractal.emit(());
                    } else if touches.length() == 2 {
                        // ピンチズーム
                        let touch1 = touches.get(0).unwrap();
                        let touch2 = touches.get(1).unwrap();
                        let dx = touch1.client_x() - touch2.client_x();
                        let dy = touch1.client_y() - touch2.client_y();
                        let distance = ((dx * dx + dy * dy) as f64).sqrt();

                        if let Some(last_distance) = *last_pinch_distance {
                            let factor = distance / last_distance;
                            let new_zoom = *zoom * factor;

                            // ピンチの中心点を計算
                            let center_x = (touch1.client_x() + touch2.client_x()) as f64 / 2.0
                                - canvas.client_left() as f64;
                            let center_y = (touch1.client_y() + touch2.client_y()) as f64 / 2.0
                                - canvas.client_top() as f64;

                            let dx = center_x / *zoom - center_x / new_zoom;
                            let dy = center_y / *zoom - center_y / new_zoom;

                            offset_x.set(*offset_x + dx);
                            offset_y.set(*offset_y + dy);
                            zoom.set(new_zoom);
                            draw_fractal.emit(());
                        }
                        last_pinch_distance.set(Some(distance));
                    }
                }
            }
        })
    };

    let on_touch_end = {
        let is_dragging = is_dragging.clone();
        let last_pinch_distance = last_pinch_distance.clone();

        Callback::from(move |event: TouchEvent| {
            is_dragging.set(false);
            last_pinch_distance.set(None);
        })
    };

    // 初期レンダリング時に描画を行う
    {
        let canvas_ref = canvas_ref.clone();
        let draw_fractal = draw_fractal.clone();
        use_effect(move || {
            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                // ウィンドウサイズに合わせる
                let window = window().unwrap();
                canvas.set_width(window.inner_width().unwrap().as_f64().unwrap() as u32);
                canvas.set_height(window.inner_height().unwrap().as_f64().unwrap() as u32);
                draw_fractal.emit(());
            }
            || ()
        });
    }

    html! {
        <div>
            <div style="position: absolute; top: 10px; left: 10px; background: rgba(255,255,255,0.7); padding: 5px; z-index: 1;">
                <div>{ format!("X: {:.3}", *offset_x) }</div>
                <div>{ format!("Y: {:.3}", *offset_y) }</div>
                <div>{ format!("Zoom: {:.1}x", *zoom / 200.0) }</div>
            </div>
            <canvas
                ref={canvas_ref}
                onmousedown={on_mouse_down}
                onmousemove={on_mouse_move}
                onmouseup={on_mouse_up}
                onwheel={on_wheel}
                ontouchstart={on_touch_start}
                ontouchmove={on_touch_move}
                ontouchend={on_touch_end}
            ></canvas>
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
