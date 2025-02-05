use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, MouseEvent, WheelEvent};
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

    // 初期レンダリング時に描画を行う
    {
        let draw_fractal = draw_fractal.clone();
        use_effect(move || {
            draw_fractal.emit(());
            || ()
        });
    }

    html! {
        <div>
            <div style="position: absolute; top: 10px; left: 10px; background: rgba(255,255,255,0.7); padding: 5px;">
                <div>{ format!("X: {:.3}", *offset_x) }</div>
                <div>{ format!("Y: {:.3}", *offset_y) }</div>
                <div>{ format!("Zoom: {:.1}x", *zoom / 200.0) }</div>
            </div>
            <canvas
                ref={canvas_ref}
                width=800
                height=600
                onmousedown={on_mouse_down}
                onmousemove={on_mouse_move}
                onmouseup={on_mouse_up}
                onwheel={on_wheel}
            ></canvas>
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
