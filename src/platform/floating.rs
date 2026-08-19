// SPDX-License-Identifier: MIT
//
// The portable backend: macOS, Windows, X11 and GNOME.
//
// None of these can make part of a window click-through the way a layer-shell
// input region can, so instead of one strip-wide surface this keeps a small
// always-on-top window that *is* the crab and moves it along the bottom of the
// screen. Everything outside the window is untouched by construction, which is
// the same end result by simpler means.
//
// Presentation goes through wgpu rather than a software blit: the sprite is
// mostly transparent, and softbuffer's pixel format reserves no alpha channel.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId, WindowLevel};

use crate::app::{Button, Core};
use crate::geom::IRect;

pub fn run(core: Core) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut builder = EventLoop::builder();

    // Keep the crab out of the Dock and the ⌘-Tab switcher: it is a companion,
    // not an application the user switches to.
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }

    let event_loop = builder.build().map_err(|e| format!("cannot create an event loop: {e}"))?;
    // Poll rather than Wait: the crab animates continuously.
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = FloatingApp { core, window: None, gpu: None, strip_origin: (0, 0), placed: IRect::EMPTY };
    event_loop.run_app(&mut app).map_err(|e| format!("event loop failed: {e}"))
}

struct FloatingApp {
    core: Core,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    /// Top-left of the virtual strip, in physical screen coordinates.
    strip_origin: (i32, i32),
    /// Where the window currently is, in strip coordinates.
    placed: IRect,
}

impl ApplicationHandler for FloatingApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = Window::default_attributes()
            .with_title("Claude Crab")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            // Sized and positioned properly on the first frame; showing it
            // before then would flash it in the wrong place.
            .with_visible(false)
            .with_inner_size(PhysicalSize::new(64, 64));

        #[cfg(target_os = "windows")]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_skip_taskbar(true)
        };

        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                log::error!("cannot create a window: {err}");
                event_loop.exit();
                return;
            }
        };

        // Work out the strip from whichever monitor the window landed on.
        //
        // Wayland reports neither a current nor a primary monitor for a window
        // that has not been mapped yet, so fall through to the first one the
        // event loop knows about rather than guessing at a resolution.
        let monitor = window
            .current_monitor()
            .or_else(|| window.primary_monitor())
            .or_else(|| event_loop.available_monitors().next());
        let scale = monitor.as_ref().map_or(1.0, |m| m.scale_factor()) as f32;
        let (mon_pos, mon_size) = match monitor.as_ref() {
            Some(m) => (m.position(), m.size()),
            None => {
                log::warn!("no monitor reported; assuming 1920x1080 at the origin");
                (PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080))
            }
        };

        let strip_h = (self.core.logical_strip_height() as f32 * scale).round() as i32;
        let margin = (self.core.config.bottom_margin as f32 * scale).round() as i32;
        self.strip_origin =
            (mon_pos.x, mon_pos.y + mon_size.height as i32 - strip_h - margin);
        self.core.set_geometry(mon_size.width as i32, strip_h, scale);
        log::info!(
            "floating strip: {}x{} at {:?}, scale {scale}",
            mon_size.width,
            strip_h,
            self.strip_origin
        );

        // Wayland deliberately gives clients no way to position their own
        // windows, so on GNOME the crab animates where the compositor puts it
        // instead of walking. Say so once, rather than leaving the user to
        // wonder why it is stuck.
        #[cfg(all(unix, not(target_os = "macos")))]
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            log::warn!(
                "Wayland does not let a client position its own window, so the crab \
                 will animate in place rather than walk across the screen. A compositor \
                 with wlr-layer-shell (Sway, Hyprland, KWin, Wayfire, River) gets the \
                 full effect."
            );
        }

        match Gpu::new(window.clone()) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(err) => {
                log::error!("{err}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => event_loop.exit(),

            WindowEvent::CursorMoved { position, .. } => {
                // Window-relative physical pixels -> strip coordinates.
                self.core.pointer_moved(
                    self.placed.x as f32 + position.x as f32,
                    self.placed.y as f32 + position.y as f32,
                );
            }
            WindowEvent::CursorLeft { .. } => self.core.pointer_left(),

            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => Button::Other,
                };
                match state {
                    ElementState::Pressed => self.core.pointer_pressed(button),
                    ElementState::Released => self.core.pointer_released(button),
                }
            }

            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

impl FloatingApp {
    fn draw(&mut self) {
        let Some(painted) = self.core.frame() else { return };
        let Some(window) = self.window.as_ref().cloned() else { return };
        let Some(gpu) = self.gpu.as_mut() else { return };

        let surface = painted.surface;
        let resized = surface.width != self.placed.width || surface.height != self.placed.height;
        let moved = surface.x != self.placed.x || surface.y != self.placed.y;

        if resized {
            let _ = window.request_inner_size(PhysicalSize::new(
                surface.width as u32,
                surface.height as u32,
            ));
            gpu.resize(surface.width as u32, surface.height as u32);
        }
        if moved || resized {
            window.set_outer_position(PhysicalPosition::new(
                self.strip_origin.0 + surface.x,
                self.strip_origin.1 + surface.y,
            ));
        }
        let first = self.placed.is_empty();
        self.placed = surface;

        if let Err(err) = gpu.present(self.core.pixels(), self.core.row_bytes(), surface) {
            log::error!("present failed: {err}");
            return;
        }
        if first {
            window.set_visible(true);
        }
    }
}

// --- wgpu presentation ------------------------------------------------------

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    layout: wgpu::BindGroupLayout,
    texture: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
}

impl Gpu {
    fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("cannot create a render surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("no usable GPU adapter: {e}"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("claude-crab"),
            required_features: wgpu::Features::empty(),
            // Keep the floor low: this draws one textured triangle.
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("cannot acquire a GPU device: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        // The crab is drawn premultiplied, so ask the compositor to treat it
        // that way. Falling back to Auto keeps us running (opaque) rather than
        // refusing to start on a backend without alpha compositing.
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
            log::warn!("no premultiplied alpha; the crab's edges may look wrong");
            wgpu::CompositeAlphaMode::PostMultiplied
        } else {
            log::warn!("this GPU backend cannot composite alpha; the crab will have a background");
            caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes.first().copied().unwrap_or(wgpu::PresentMode::Fifo),
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crab-blit"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crab-blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crab-blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crab-blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    // The surface is cleared and fully overwritten each frame,
                    // so there is nothing to blend against.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Nearest neighbour: this is pixel art, smoothing turns it to mush.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("crab-nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self { surface, device, queue, config, pipeline, sampler, layout, texture: None })
    }

    fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.config.width && height == self.config.height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn ensure_texture(&mut self, width: u32, height: u32) {
        if let Some((_, _, w, h)) = &self.texture
            && *w == width
            && *h == height
        {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crab-frame"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // The Skia surface's pixels() are physically RGBA (see
            // make_surface), so the texture matches; the swapchain stays
            // Bgra8Unorm and the blit converts in the sampler.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crab-frame"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.texture = Some((texture, bind_group, width, height));
    }

    fn present(&mut self, pixels: &[u8], row_bytes: usize, rect: IRect) -> Result<(), String> {
        let (width, height) = (rect.width.max(1) as u32, rect.height.max(1) as u32);
        self.resize(width, height);
        self.ensure_texture(width, height);
        let Some((texture, bind_group, _, _)) = self.texture.as_ref() else {
            return Err("no frame texture".to_string());
        };

        let needed = row_bytes * height as usize;
        if pixels.len() < needed {
            return Err(format!("frame is {} bytes, expected {needed}", pixels.len()));
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels[..needed],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes as u32),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );

        use wgpu::CurrentSurfaceTexture as Acquired;
        let frame = match self.surface.get_current_texture() {
            Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
            // The surface goes stale across a resize; reconfigure and let the
            // next frame draw. Skipping one frame of a walking crab is
            // invisible, and the alternative is presenting a torn buffer.
            Acquired::Outdated | Acquired::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            // Transient: the window is hidden, or the compositor is busy.
            Acquired::Timeout | Acquired::Occluded => return Ok(()),
            Acquired::Validation => return Err("surface validation error".to_string()),
        };

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("crab") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("crab"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

/// One oversized triangle covering the viewport, textured with the frame.
const SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    var out: VsOut;
    let x = f32((idx << 1u) & 2u);
    let y = f32(idx & 2u);
    out.uv = vec2<f32>(x, y);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(frame, frame_sampler, in.uv);
}
"#;
