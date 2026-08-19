// SPDX-License-Identifier: MIT
//
// The wlr-layer-shell backend.
//
// The window is a full-screen surface on the Top layer with a -1 exclusive
// zone: it reserves nothing and ignores the panel's reservation, so the
// walking band hugs the true bottom edge of the screen and the crab walks
// over the panel rather than hovering above it. Covering the whole screen
// rather than just a bottom strip is what lets a drag drop the crab at any
// coordinate; the input region keeps everything but the character itself
// click-through, so the rest of the surface is inert.
//
// The space above the walking band also serves as menu headroom: the
// right-click menu is painted into it rather than being a real popup, because
// an xdg_popup parented to a layer surface is far more fragile.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm, wl_surface},
};

use crate::app::{Button, Core};
use crate::geom::IRect;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Position changes below this are not worth a set_input_region plus commit.
const REGION_MOVE_THRESHOLD: i32 = 6;

/// Does this session offer wlr-layer-shell?
///
/// Probed with a throwaway connection so the caller can fall back before
/// committing to a backend.
pub fn available() -> bool {
    struct Probe;
    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    let Ok(conn) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) = registry_queue_init::<Probe>(&conn) else {
        return false;
    };
    globals
        .contents()
        .with_list(|list| list.iter().any(|g| g.interface == "zwlr_layer_shell_v1"))
}

pub fn run(core: Core) -> Result<(), String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("cannot connect to a Wayland compositor: {e}"))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| format!("cannot initialise registry: {e}"))?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).map_err(|e| format!("wl_compositor unavailable: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| format!("zwlr_layer_shell_v1 unavailable: {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm unavailable: {e}"))?;

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        compositor,
        pool: None,
        layer: None,
        width: 0,
        height: 0,
        scale: 1,
        exit: false,
        core,
        pointer: None,
        input_region: IRect::EMPTY,
        region_set: false,
    };

    // Outputs have to be known before the layer surface is created, so the
    // configured connector can be honoured.
    event_queue.roundtrip(&mut app).map_err(|e| format!("initial roundtrip failed: {e}"))?;
    app.create_layer(&qh, &layer_shell);

    while !app.exit {
        event_queue
            .blocking_dispatch(&mut app)
            .map_err(|e| format!("wayland dispatch failed: {e}"))?;
    }
    Ok(())
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,

    pool: Option<SlotPool>,
    layer: Option<LayerSurface>,

    /// Logical surface size, as configured by the compositor.
    width: u32,
    height: u32,
    scale: i32,
    exit: bool,

    core: Core,
    pointer: Option<wl_pointer::WlPointer>,
    input_region: IRect,
    region_set: bool,
}

impl App {
    fn create_layer(&mut self, qh: &QueueHandle<Self>, layer_shell: &LayerShell) {
        let output = self.pick_output();
        let surface = self.compositor.create_surface(qh);
        let layer = layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some("dev.quinnjr.claude-crab"),
            output.as_ref(),
        );

        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        // -1: ignore exclusive zones entirely. A zero zone would make the
        // compositor lift the strip above the panel's reservation, leaving the
        // panel itself a dead band the crab could never walk across. The strip
        // is created after the panel on the same layer, so it stacks above it,
        // and the input region keeps everything but the character itself
        // click-through.
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // Zero in both axes: anchored to all four edges, the compositor hands
        // us the full output.
        layer.set_size(0, 0);
        layer.commit();

        self.layer = Some(layer);
    }

    fn pick_output(&self) -> Option<wl_output::WlOutput> {
        let wanted = self.core.config.output.clone();
        if wanted.is_empty() {
            return None;
        }
        for output in self.output_state.outputs() {
            let info = self.output_state.info(&output);
            if info.as_ref().and_then(|i| i.name.as_deref()) == Some(wanted.as_str()) {
                return Some(output);
            }
        }
        log::warn!("output {wanted} not found; falling back to the default");
        None
    }

    fn device_size(&self) -> (i32, i32) {
        (self.width as i32 * self.scale, self.height as i32 * self.scale)
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let (w, h) = self.device_size();
        if w <= 0 || h <= 0 {
            return;
        }
        self.core.set_geometry(w, h, self.scale as f32);

        let Some(layer) = self.layer.as_ref() else { return };
        let surface = layer.wl_surface().clone();

        if let Some(painted) = self.core.frame() {
            self.commit_pixels(&painted.damage, w, h);
            self.update_input_region(qh);
        }

        // Ask for the next frame before committing, so the callback is part of
        // this commit.
        surface.frame(qh, FrameCallbackData(surface.clone()));
        surface.commit();
    }

    fn commit_pixels(&mut self, damage: &[IRect], w: i32, h: i32) {
        if damage.is_empty() {
            return;
        }
        let stride = w * 4;
        if self.pool.is_none() {
            match SlotPool::new((stride * h) as usize, &self.shm) {
                Ok(pool) => self.pool = Some(pool),
                Err(err) => {
                    log::error!("cannot create shm pool: {err}");
                    return;
                }
            }
        }
        let Some(pool) = self.pool.as_mut() else { return };

        let (buffer, canvas) = match pool.create_buffer(w, h, stride, wl_shm::Format::Argb8888) {
            Ok(pair) => pair,
            Err(err) => {
                log::error!("cannot create shm buffer: {err}");
                return;
            }
        };

        // ponytail: the whole surface is copied into the freshly-acquired slot
        // rather than only the damaged rows. SlotPool may hand back a different
        // slot each frame, and a partial copy into a slot holding a stale frame
        // would tear. At 4MB for a 4K strip this is well under a millisecond;
        // add per-slot damage tracking if it ever shows up in a profile.
        //
        // The renderer's pixels are physically RGBA (see make_surface), but
        // wl_shm's ARGB8888 wants little-endian BGRA bytes, so the copy swaps
        // R and B on the way through.
        let src = self.core.pixels();
        let src_stride = self.core.row_bytes();
        let dst_stride = stride as usize;
        if src_stride == dst_stride && src.len() >= canvas.len() {
            let len = canvas.len();
            copy_rgba_to_bgra(&src[..len], canvas);
        } else {
            for row in 0..h as usize {
                let (s, d) = (row * src_stride, row * dst_stride);
                if s + dst_stride <= src.len() && d + dst_stride <= canvas.len() {
                    copy_rgba_to_bgra(&src[s..s + dst_stride], &mut canvas[d..d + dst_stride]);
                }
            }
        }

        let Some(layer) = self.layer.as_ref() else { return };
        let surface = layer.wl_surface();
        for rect in damage {
            // Damage is tracked in device pixels, which is what damage_buffer
            // takes -- no scale conversion needed.
            surface.damage_buffer(rect.x, rect.y, rect.width, rect.height);
        }
        if let Err(err) = buffer.attach_to(surface) {
            log::error!("cannot attach buffer: {err}");
        }
    }

    /// Keep the input region in step with the character.
    ///
    /// The surface is otherwise click-through. Rather than a global flag, the
    /// input region is the character's own rectangle, so a right click on the
    /// crab opens its menu while every other pixel of the strip still passes
    /// clicks to whatever is underneath.
    fn update_input_region(&mut self, qh: &QueueHandle<Self>) {
        let wanted = self.core.input_rect();

        // Size changes always apply; position changes only past a threshold, so
        // a walking character does not commit a new input region every frame.
        let resized =
            wanted.width != self.input_region.width || wanted.height != self.input_region.height;
        let moved = (wanted.x - self.input_region.x).abs() + (wanted.y - self.input_region.y).abs()
            >= REGION_MOVE_THRESHOLD;
        if self.region_set && !resized && !moved {
            return;
        }
        self.input_region = wanted;
        self.region_set = true;

        let Some(layer) = self.layer.as_ref() else { return };
        let region = self.compositor.wl_compositor().create_region(qh, ());
        if !wanted.is_empty() {
            // wl_region speaks logical coordinates.
            region.add(
                wanted.x / self.scale,
                wanted.y / self.scale,
                wanted.width / self.scale,
                wanted.height / self.scale,
            );
        }
        layer.wl_surface().set_input_region(Some(&region));
        region.destroy();
    }
}

/// Copy RGBA-ordered pixels into a little-endian ARGB8888 (BGRA-byte) canvas.
fn copy_rgba_to_bgra(src: &[u8], dst: &mut [u8]) {
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
        d[3] = s[3];
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if new_factor == self.scale || new_factor <= 0 {
            return;
        }
        self.scale = new_factor;
        if let Some(layer) = self.layer.as_ref() {
            layer.wl_surface().set_buffer_scale(new_factor);
        }
        // The pool is sized for the old buffer; drop it so the next frame
        // allocates one that fits.
        self.pool = None;
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.draw(qh);
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        log::info!("compositor closed the layer surface");
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (mut width, mut height) = configure.new_size;
        // A zero dimension means "you choose"; the strip wants the full width.
        if width == 0 {
            width = 1920;
            log::warn!("compositor left the width to us; assuming {width}");
        }
        if height == 0 {
            height = 1080;
            log::warn!("compositor left the height to us; assuming {height}");
        }
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        // A resize invalidates the pool's sizing assumptions.
        self.pool = None;
        self.draw(qh);
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(err) => log::warn!("cannot acquire pointer: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let scale = self.scale as f32;
        for event in events {
            // Pointer positions arrive in logical coordinates.
            let (x, y) = (event.position.0 as f32 * scale, event.position.1 as f32 * scale);
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.core.pointer_moved(x, y);
                }
                PointerEventKind::Leave { .. } => self.core.pointer_left(),
                PointerEventKind::Press { button, .. } => {
                    self.core.pointer_moved(x, y);
                    self.core.pointer_pressed(match button {
                        BTN_LEFT => Button::Left,
                        BTN_RIGHT => Button::Right,
                        _ => Button::Other,
                    });
                }
                PointerEventKind::Release { button, .. } => {
                    self.core.pointer_released(match button {
                        BTN_LEFT => Button::Left,
                        BTN_RIGHT => Button::Right,
                        _ => Button::Other,
                    });
                }
                PointerEventKind::Axis { .. } => {}
            }
        }
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(App);

#[cfg(test)]
mod tests {
    use super::copy_rgba_to_bgra;

    #[test]
    fn the_copy_swaps_red_and_blue() {
        // Terracotta in RGBA must land as BGRA in the shm buffer; alpha stays
        // at byte 3 in both orders.
        let src = [208u8, 106, 75, 255, 0, 0, 0, 0];
        let mut dst = [0u8; 8];
        copy_rgba_to_bgra(&src, &mut dst);
        assert_eq!(dst, [75, 106, 208, 255, 0, 0, 0, 0]);
    }
}
delegate_noop!(App: ignore wl_region::WlRegion);
smithay_client_toolkit::delegate_dispatch2!(App);
