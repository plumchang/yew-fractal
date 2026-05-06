// Rc + RefCell は Rust + WASM で「複数所有 + 内部書き換え」を行う定番パターン。
//   - Rc<T>: 参照カウントによる共有所有権（シングルスレッド向け）
//   - RefCell<T>: 不変参照しか取れないものに対する「内部可変性」を提供
// JS の世界はシングルスレッドなので Rc で十分（Arc は不要）。
use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    window, Blob, BlobPropertyBag, CanvasRenderingContext2d, HtmlCanvasElement, MessageEvent,
    MouseEvent, Performance, TouchEvent, Url, WheelEvent, Worker,
};

/// 現在時刻 (ms) を取得するユーティリティ。
/// `window.performance.now()` を Rust から呼んでいる。
/// `and_then`/`map` は Option のメソッドチェーンで、JS の Optional Chaining (`?.`) に近い。
fn now_ms() -> f64 {
    window()
        .and_then(|w| w.performance())
        .map(|p: Performance| p.now())
        .unwrap_or(0.0)
}

/// 計測値。`#[derive]` でクローン可能・コピー可能・デフォルト値あり・等値比較可能を自動実装。
/// `Copy` が付いているので所有権の移動ではなくビット単位コピーになる（軽量）。
#[derive(Clone, Copy, Default, PartialEq)]
struct Metrics {
    last_frame_ms: f64,
    fps: f64,
}
use yew::prelude::*;

const NUM_WORKERS: usize = 4;
const MAX_ITER: u32 = 100;

/// Trunk が `data-type="worker"` で出力した Worker を Blob 経由で起動する。
/// 出力ファイル名は `<bin名>.js` / `<bin名>_bg.wasm` で固定（ハッシュなし）。
/// `document.baseURI` を基準に絶対 URL を組み立てるので、
/// GitHub Pages のサブパス（例：`/yew-fractal/`）配信でも動作する。
fn spawn_worker() -> Worker {
    let base_uri = window()
        .unwrap()
        .document()
        .unwrap()
        .base_uri()
        .unwrap()
        .unwrap();
    let js_url = Url::new_with_base("fractal-worker.js", &base_uri)
        .unwrap()
        .href();
    let wasm_url = Url::new_with_base("fractal-worker_bg.wasm", &base_uri)
        .unwrap()
        .href();
    let script = Array::new();
    script.push(&format!(r#"importScripts("{js_url}");wasm_bindgen("{wasm_url}");"#).into());
    let bag = BlobPropertyBag::new();
    bag.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&script, &bag).unwrap();
    let url = Url::create_object_url_with_blob(&blob).unwrap();
    Worker::new(&url).expect("failed to spawn worker")
}

/// 4 つの Worker を束ねる構造体。React 版の FractalPool クラスと同等の役割。
///
/// 構造体の中身を変更したい時、Yew の Callback は `Fn` を求めるため `&mut self` を取れない。
/// そこで `Rc<RefCell<Pool>>` でラップし、複数の Callback から借用 → 一時的に書き換え可能にする。
/// このパターンが Rust + WASM の UI 状態管理の主軸になる。
struct Pool {
    workers: Vec<Worker>,
    /// "ready" 通知を受け取った Worker の数。NUM_WORKERS に達するまで描画は保留する。
    /// Rust + WASM Worker は wasm 初期化が非同期なので必要（Step 4 参照）
    ready_count: usize,
    canvas: Option<HtmlCanvasElement>,
    ctx: Option<CanvasRenderingContext2d>,
    width: u32,
    height: u32,
    img_buf: Vec<u8>,
    received: usize,
    in_flight: bool,
    /// 描画中に新しいリクエストが来た場合の保留パラメータ（最後の1つだけ保持）
    queued: Option<(f64, f64, f64)>,
    /// 計測用：現フレームの開始時刻 (ms)
    frame_start: f64,
    /// 計測用：直近フレーム完了時刻のリングバッファ（FPS 計算用）
    frame_history: Vec<f64>,
    /// 計測値の通知先（Yew state setter）
    metrics_setter: Option<UseStateHandle<Metrics>>,
    /// Worker -> Pool 用クロージャ（drop 防止のため保持）
    _onmsg: Vec<Closure<dyn FnMut(MessageEvent)>>,
}

impl Pool {
    /// Pool を生成し、Worker × 4 を起動して onmessage を登録する。
    /// 戻り値が `Rc<RefCell<Self>>` なのは、各 Worker のコールバックから自分自身を
    /// 共有所有 + 内部書き換えするため。
    fn new() -> Rc<RefCell<Self>> {
        let pool = Rc::new(RefCell::new(Pool {
            workers: Vec::with_capacity(NUM_WORKERS),
            ready_count: 0,
            canvas: None,
            ctx: None,
            width: 0,
            height: 0,
            img_buf: Vec::new(),
            received: 0,
            in_flight: false,
            queued: None,
            frame_start: 0.0,
            frame_history: Vec::with_capacity(32),
            metrics_setter: None,
            _onmsg: Vec::with_capacity(NUM_WORKERS),
        }));

        for _ in 0..NUM_WORKERS {
            let worker = spawn_worker();
            // pool 自身を各クロージャに「共有」するために clone（参照カウントが増えるだけ）
            let pool_ref = pool.clone();
            // Step 4 で説明した Closure::wrap パターン。
            // クロージャは MessageEvent を受け取って Pool::on_message に転送する。
            let onmessage = Closure::wrap(Box::new(move |msg: MessageEvent| {
                Pool::on_message(&pool_ref, msg);
            }) as Box<dyn FnMut(MessageEvent)>);
            worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            // ブロックスコープで borrow_mut の生存期間を限定（次のループ反復で再借用するため）
            {
                let mut p = pool.borrow_mut();
                p.workers.push(worker);
                // クロージャは pool に保持しないと Drop されて onmessage が無効になる。
                // Step 4 では forget() を使ったが、ここでは Pool が生きている限り
                // クロージャも生きるよう Vec で保持している。
                p._onmsg.push(onmessage);
            }
        }

        pool
    }

    fn attach_canvas(&mut self, canvas: HtmlCanvasElement) {
        let ctx = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()
            .unwrap();
        self.canvas = Some(canvas);
        self.ctx = Some(ctx);
    }

    fn submit(&mut self, zoom: f64, offset_x: f64, offset_y: f64) {
        // Worker が全員 ready するまでは保留
        if self.ready_count < NUM_WORKERS {
            self.queued = Some((zoom, offset_x, offset_y));
            return;
        }
        if self.in_flight {
            self.queued = Some((zoom, offset_x, offset_y));
            return;
        }
        let canvas = match &self.canvas {
            Some(c) => c,
            None => return,
        };
        let width = canvas.width();
        let height = canvas.height();
        if width == 0 || height == 0 {
            return;
        }

        self.width = width;
        self.height = height;
        self.img_buf = vec![0u8; (width * height * 4) as usize];
        self.received = 0;
        self.in_flight = true;
        self.frame_start = now_ms();

        let chunk = (height as usize) / NUM_WORKERS;
        for (i, worker) in self.workers.iter().enumerate() {
            let start_y = i * chunk;
            let end_y = if i == NUM_WORKERS - 1 {
                height as usize
            } else {
                (i + 1) * chunk
            };
            let msg = Object::new();
            let _ = Reflect::set(&msg, &"width".into(), &JsValue::from(width));
            let _ = Reflect::set(&msg, &"startY".into(), &JsValue::from(start_y as u32));
            let _ = Reflect::set(&msg, &"endY".into(), &JsValue::from(end_y as u32));
            let _ = Reflect::set(&msg, &"zoom".into(), &JsValue::from(zoom));
            let _ = Reflect::set(&msg, &"offsetX".into(), &JsValue::from(offset_x));
            let _ = Reflect::set(&msg, &"offsetY".into(), &JsValue::from(offset_y));
            let _ = Reflect::set(&msg, &"maxIter".into(), &JsValue::from(MAX_ITER));
            worker.post_message(&msg.into()).expect("post_message");
        }
    }

    /// Worker からのメッセージを処理する。`&Rc<RefCell<Self>>` を取るのは、
    /// 借用を解放してから submit を再度呼びたいケース（queued の処理）があるため。
    /// `&mut self` だと「借用中に再度 borrow_mut」になり panic する。
    fn on_message(pool_ref: &Rc<RefCell<Self>>, msg: MessageEvent) {
        let data = msg.data();

        // ready 通知の判定
        if let Ok(t) = Reflect::get(&data, &"type".into()) {
            if t.as_string().as_deref() == Some("ready") {
                let mut p = pool_ref.borrow_mut();
                p.ready_count += 1;
                if p.ready_count == NUM_WORKERS {
                    if let Some((z, ox, oy)) = p.queued.take() {
                        drop(p);
                        pool_ref.borrow_mut().submit(z, ox, oy);
                    }
                }
                return;
            }
        }

        let start_y = Reflect::get(&data, &"startY".into())
            .ok()
            .and_then(|v| v.as_f64())
            .map(|v| v as u32);
        let chunk_buf = Reflect::get(&data, &"chunkData".into()).ok();

        if let (Some(start_y), Some(buf)) = (start_y, chunk_buf) {
            let arr = Uint8Array::new(&buf);
            let mut p = pool_ref.borrow_mut();
            let width = p.width as usize;
            let offset = (start_y as usize) * width * 4;
            let len = arr.length() as usize;
            // ウィンドウサイズが変わって submit 中の場合に備えてガード
            if offset + len <= p.img_buf.len() {
                arr.copy_to(&mut p.img_buf[offset..offset + len]);
            }
            p.received += 1;

            if p.received == NUM_WORKERS {
                p.in_flight = false;
                p.received = 0;
                if let (Some(ctx), w, h) = (&p.ctx, p.width, p.height) {
                    let clamped = wasm_bindgen::Clamped(p.img_buf.as_slice());
                    if let Ok(image) = web_sys::ImageData::new_with_u8_clamped_array(clamped, w) {
                        let _ = ctx.put_image_data(&image, 0.0, 0.0);
                        let _ = h;
                    }
                }

                // 計測値の更新
                let end = now_ms();
                let frame_ms = end - p.frame_start;
                p.frame_history.push(end);
                let cutoff = end - 1000.0;
                p.frame_history.retain(|t| *t >= cutoff);
                let fps = p.frame_history.len() as f64;
                if let Some(setter) = &p.metrics_setter {
                    setter.set(Metrics {
                        last_frame_ms: frame_ms,
                        fps,
                    });
                }

                if let Some((z, ox, oy)) = p.queued.take() {
                    drop(p);
                    pool_ref.borrow_mut().submit(z, ox, oy);
                }
            }
        }
    }
}

/// Yew の関数コンポーネント。React の関数コンポーネントとほぼ同じ思想。
///   - `use_state` で状態を保持（再レンダー時に値が保存される）
///   - `use_node_ref` で DOM 要素への参照を保持
///   - `use_mut_ref` で「再レンダー間で生き残る可変な値」を保持（React の useRef 相当）
///   - `use_effect_with` で副作用（マウント時の初期化など）
#[function_component]
fn App() -> Html {
    let canvas_ref = use_node_ref();
    // use_state は React の useState と同じ。set した時に再レンダーが起きる。
    let zoom = use_state(|| 200.0_f64);
    let offset_x = use_state(|| -2.0_f64);
    let offset_y = use_state(|| -1.0_f64);
    let is_dragging = use_state(|| false);
    let last_pos = use_state(|| (0.0_f64, 0.0_f64));
    let last_pinch = use_state(|| None::<f64>);
    let metrics = use_state(Metrics::default);

    // Pool は初回マウント時に1度だけ生成。React の useRef 同様、再レンダーで作り直されない。
    // `Option<...>` にしているのは、初期化前は None にしておきたいため。
    let pool = use_mut_ref(|| Option::<Rc<RefCell<Pool>>>::None);

    // Yew の Callback を作る定型パターン:
    //   1. `let xxx = xxx.clone();` でブロック内に複製を持ち込む
    //   2. `Callback::from(move |...| { ... })` で move キャプチャ
    // これは Step 4 で説明した「move したら外でも使えなくなる」問題への対処。
    // React の paramsRef パターンに相当するが、Yew ではこの clone 連打が定番。
    let request_draw = {
        let pool = pool.clone();
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        Callback::from(move |_: ()| {
            // pool は use_mut_ref なので RefCell。borrow() で読み取り専用借用。
            // その中身（Option<Rc<RefCell<Pool>>>）の Some をパターンマッチで取り出し、
            // borrow_mut() で Pool 本体を可変借用する。借用が二重に見えるが、
            // 別々の RefCell なので問題ない。
            if let Some(p) = pool.borrow().as_ref() {
                // *zoom は UseStateHandle<T> の Deref で T への参照を取り出す記法
                p.borrow_mut().submit(*zoom, *offset_x, *offset_y);
            }
        })
    };

    // 初回マウント時の副作用。React の `useEffect(() => {...}, [])` 相当。
    // 第1引数 `()` は依存配列が空という意味で、コンポーネント生存中に1度だけ実行される。
    {
        let canvas_ref = canvas_ref.clone();
        let pool = pool.clone();
        let request_draw = request_draw.clone();
        let metrics = metrics.clone();
        use_effect_with((), move |_| {
            // Pool 初期化（Worker × 4 を起動）
            let p = Pool::new();
            // metrics の setter を Pool に渡しておく。Pool 内で計算が完了したら
            // この setter 経由で UI を更新する（React 版の onMetrics と同じ仕組み）
            p.borrow_mut().metrics_setter = Some(metrics);
            *pool.borrow_mut() = Some(p.clone());

            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                let win = window().unwrap();
                canvas.set_width(win.inner_width().unwrap().as_f64().unwrap() as u32);
                canvas.set_height(win.inner_height().unwrap().as_f64().unwrap() as u32);
                p.borrow_mut().attach_canvas(canvas);
            }
            request_draw.emit(());
            || ()
        });
    }

    let canvas_pos = {
        let canvas_ref = canvas_ref.clone();
        move |client_x: f64, client_y: f64| -> (f64, f64) {
            if let Some(canvas) = canvas_ref.cast::<HtmlCanvasElement>() {
                let rect = canvas.get_bounding_client_rect();
                (client_x - rect.left(), client_y - rect.top())
            } else {
                (client_x, client_y)
            }
        }
    };

    let on_mouse_down = {
        let is_dragging = is_dragging.clone();
        let last_pos = last_pos.clone();
        let canvas_pos = canvas_pos.clone();
        Callback::from(move |e: MouseEvent| {
            let (x, y) = canvas_pos(e.client_x() as f64, e.client_y() as f64);
            is_dragging.set(true);
            last_pos.set((x, y));
            e.prevent_default();
        })
    };

    let on_mouse_move = {
        let is_dragging = is_dragging.clone();
        let last_pos = last_pos.clone();
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let request_draw = request_draw.clone();
        let canvas_pos = canvas_pos.clone();
        Callback::from(move |e: MouseEvent| {
            if !*is_dragging {
                return;
            }
            let (x, y) = canvas_pos(e.client_x() as f64, e.client_y() as f64);
            let (lx, ly) = *last_pos;
            offset_x.set(*offset_x - (x - lx) / *zoom);
            offset_y.set(*offset_y - (y - ly) / *zoom);
            last_pos.set((x, y));
            request_draw.emit(());
        })
    };

    let on_mouse_up = {
        let is_dragging = is_dragging.clone();
        Callback::from(move |_: MouseEvent| {
            is_dragging.set(false);
        })
    };

    let on_wheel = {
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let request_draw = request_draw.clone();
        let canvas_pos = canvas_pos.clone();
        Callback::from(move |e: WheelEvent| {
            e.prevent_default();
            let (mx, my) = canvas_pos(e.client_x() as f64, e.client_y() as f64);
            let factor = (0.999_f64).powf(e.delta_y());
            let new_zoom = *zoom * factor;
            let dx = mx / *zoom - mx / new_zoom;
            let dy = my / *zoom - my / new_zoom;
            offset_x.set(*offset_x + dx);
            offset_y.set(*offset_y + dy);
            zoom.set(new_zoom);
            request_draw.emit(());
        })
    };

    let on_touch_start = {
        let canvas_pos = canvas_pos.clone();
        let is_dragging = is_dragging.clone();
        let last_pos = last_pos.clone();
        let last_pinch = last_pinch.clone();
        Callback::from(move |e: TouchEvent| {
            e.prevent_default();
            let touches = e.touches();
            if touches.length() == 1 {
                let t = touches.get(0).unwrap();
                let (x, y) = canvas_pos(t.client_x() as f64, t.client_y() as f64);
                is_dragging.set(true);
                last_pos.set((x, y));
                last_pinch.set(None);
            } else if touches.length() == 2 {
                let t1 = touches.get(0).unwrap();
                let t2 = touches.get(1).unwrap();
                let dx = (t1.client_x() - t2.client_x()) as f64;
                let dy = (t1.client_y() - t2.client_y()) as f64;
                last_pinch.set(Some((dx * dx + dy * dy).sqrt()));
            }
        })
    };

    let on_touch_move = {
        let canvas_pos = canvas_pos.clone();
        let is_dragging = is_dragging.clone();
        let last_pos = last_pos.clone();
        let last_pinch = last_pinch.clone();
        let zoom = zoom.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let request_draw = request_draw.clone();
        Callback::from(move |e: TouchEvent| {
            e.prevent_default();
            let touches = e.touches();
            if touches.length() == 1 && *is_dragging {
                let t = touches.get(0).unwrap();
                let (x, y) = canvas_pos(t.client_x() as f64, t.client_y() as f64);
                let (lx, ly) = *last_pos;
                offset_x.set(*offset_x - (x - lx) / *zoom);
                offset_y.set(*offset_y - (y - ly) / *zoom);
                last_pos.set((x, y));
                request_draw.emit(());
            } else if touches.length() == 2 {
                let t1 = touches.get(0).unwrap();
                let t2 = touches.get(1).unwrap();
                let dx = (t1.client_x() - t2.client_x()) as f64;
                let dy = (t1.client_y() - t2.client_y()) as f64;
                let dist = (dx * dx + dy * dy).sqrt();
                if let Some(prev) = *last_pinch {
                    let factor = dist / prev;
                    let new_zoom = *zoom * factor;
                    let cx = (t1.client_x() + t2.client_x()) as f64 / 2.0;
                    let cy = (t1.client_y() + t2.client_y()) as f64 / 2.0;
                    let (cx, cy) = canvas_pos(cx, cy);
                    let dx = cx / *zoom - cx / new_zoom;
                    let dy = cy / *zoom - cy / new_zoom;
                    offset_x.set(*offset_x + dx);
                    offset_y.set(*offset_y + dy);
                    zoom.set(new_zoom);
                    request_draw.emit(());
                }
                last_pinch.set(Some(dist));
            }
        })
    };

    let on_touch_end = {
        let is_dragging = is_dragging.clone();
        let last_pinch = last_pinch.clone();
        Callback::from(move |e: TouchEvent| {
            e.prevent_default();
            is_dragging.set(false);
            last_pinch.set(None);
        })
    };

    html! {
        <div>
            <div style="position: absolute; top: 10px; left: 10px; background: rgba(255,255,255,0.7); padding: 5px; z-index: 1;">
                <div>{ format!("X: {:.3}", *offset_x) }</div>
                <div>{ format!("Y: {:.3}", *offset_y) }</div>
                <div>{ format!("Zoom: {:.1}x", *zoom / 200.0) }</div>
                <div>{ format!("Frame: {:.1} ms", metrics.last_frame_ms) }</div>
                <div>{ format!("FPS: {:.0}", metrics.fps) }</div>
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
    console_error_panic_hook::set_once();
    yew::Renderer::<App>::new().render();
}
