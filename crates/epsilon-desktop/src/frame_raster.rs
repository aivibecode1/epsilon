//! Rasterise a real egui frame **offscreen**, to pixels.
//!
//! # Why this exists
//!
//! **No wgpu backend presents on this machine**, so nothing in the repository had ever
//! produced an image of this window. The measurement is in
//! `audit/evidence/desktop-window-measurement-2026-10-03.md`: the content area stayed
//! blank white, and stayed blank white again after the whole content rect was filled
//! solid red. Every claim about this window's pixels was therefore inferred from egui's
//! source and the crate's own recorded geometry.
//!
//! A swapchain is what was failing, not a GPU. This takes the other path: it asks wgpu
//! for a **device and an offscreen texture**, renders the frame's own tessellation into
//! it, and reads the texture back. No window, no surface, no present. The two GPUs on
//! this box (`Intel UHD Graphics 620`, `AMD Radeon R7 M340`) are reachable by exactly
//! this route.
//!
//! # What it is and is not
//!
//! It renders **the same pixels the window would draw**: the same `egui::FullOutput` the
//! real `paint` produces, the same tessellation egui's own `tessellate` performs, the
//! same `egui_wgpu::Renderer` shader `eframe` uses, and the same clear colour. What it is
//! not is a screenshot of a composited desktop window — there is no OS chrome, no window
//! shadow, and no cursor. **That is a smaller gap than "no image at all" and it is still a
//! gap**, and the honest description of what a reader is looking at is "this window's draw
//! output", not "this window on screen".
//!
//! # Why it is test-only
//!
//! It needs `wgpu`, `egui-wgpu` and `png` — all three already in `Cargo.lock`, added as
//! **dev-dependencies** so none of them reaches `epsilon-desktop`'s binary. It exists to
//! be *run*, and the thing that runs it is the battery.

use std::path::Path;

/// Rasterise `full` into RGBA8 pixels, row-major, top row first.
///
/// `size_in_pixels` is the frame's size in physical pixels; it comes from the
/// `FullOutput` rather than from the window, so a caller cannot ask for an image of a size
/// the frame was not drawn at.
///
/// **Takes the `FullOutput` by value and consumes its texture deltas.** egui asserts, on
/// drop, that every delta was applied (`epaint-0.36.2/src/textures.rs:337`), and this
/// function applies them — so the ones that *were* applied must be cleared here rather than
/// left for a caller to remember. The alternative is a borrow, which cannot clear them and
/// leaves every call site with a tripwire it will eventually trip.
pub fn rasterize(
    ctx: &egui::Context,
    full: egui::FullOutput,
    clear: egui::Color32,
) -> Result<Vec<u8>, String> {
    let size = full.pixels_per_point
        * full.shapes[..]
            .iter()
            .fold(egui::Rect::ZERO, |acc, s| acc.union(s.clip_rect))
            .size();
    let width = size.x.round().max(1.0) as u32;
    let height = size.y.round().max(1.0) as u32;

    // `tokio::Runtime::block_on` drives any `Future`, and wgpu's adapter/device futures
    // are plain `std::task` ones, so this is the runtime's *scheduler* being borrowed
    // rather than a new dependency. `pollster` would do the same job and would be a new
    // crate in the graph for one call.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| format!("no runtime: {e}"))?
        .block_on(rasterize_async(ctx, full, clear, width, height))
}

async fn rasterize_async(
    ctx: &egui::Context,
    full: egui::FullOutput,
    clear: egui::Color32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let mut full = full;
    // "without display handle" is the honest descriptor here: there is no window and no
    // surface, which is the entire point — the swapchain is what was failing. `_from_env`
    // lets `WGPU_BACKEND` pick a backend on a machine with two half-working drivers.
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

    // **Try the real adapter, then the fallback one.** The order is not a preference: on a
    // machine whose only usable adapter is a software rasteriser (WARP, llvmpipe, lavapipe)
    // asking for the fallback *first* wastes the only adapter that works. Reporting which
    // one answered is the difference between "no GPU" and "no GPU *that one*", which are
    // different problems.
    let mut adapter = None;
    for force_fallback in [false, true] {
        match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: force_fallback,
                // Not a fingerprinting defence here: this process loads no untrusted
                // content, and bucketing would only make the adapter's real limits a
                // thing to discover by failure.
                apply_limit_buckets: false,
            })
            .await
        {
            Ok(a) => {
                eprintln!(
                    "frame_raster: adapter = {:?} ({:?}), fallback={force_fallback}",
                    a.get_info().name,
                    a.get_info().backend
                );
                adapter = Some(a);
                break;
            }
            Err(e) => {
                eprintln!("frame_raster: adapter request failed ({e}), fallback={force_fallback}")
            }
        }
    }
    let adapter = adapter.ok_or("no wgpu adapter: real and fallback both failed")?;

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("epsilon frame raster"),
            // `adapter.limits()` rather than `Limits::default()`: the default set asks for
            // limits some adapters do not have, and the failure is "unsupported", not
            // "too big" — which is exactly the class of surprise this file exists to
            // remove.
            required_limits: adapter.limits(),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("no device: {e}"))?;
    eprintln!("frame_raster: device created");

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("epsilon frame target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        // **No `TEXTURE_BINDING`.** Nothing samples this texture; it exists to be a render
        // target and a copy source, and asking for a binding the readback never uses is one
        // more way for an adapter to say no.
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    eprintln!("frame_raster: target texture {width}x{height} {format:?}");

    // **Compiling the shader is the step most likely to fail on a driver that cannot
    // present.** `Renderer::new` builds a module from egui's own WGSL and asks for a
    // pipeline layout; a driver with a broken or absent shader compiler can take the whole
    // process down here rather than returning an error, so this line is the one that says
    // which step ended the run.
    let mut renderer =
        egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
    eprintln!("frame_raster: egui_wgpu renderer built");

    // The order here is egui-wgpu's own (`winit.rs:566-587`), not a guess: textures, then
    // buffers, then the render pass. Uploading buffers before the font atlas exists
    // produces geometry pointing at glyphs that were never uploaded — a frame that renders
    // *something* with the text missing, which is the worst possible failure for a probe
    // whose whole job is to show the text.
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point: full.pixels_per_point,
    };
    let primitives = ctx.tessellate(full.shapes.clone(), full.pixels_per_point);
    eprintln!(
        "frame_raster: {} clipped primitives, {} texture deltas",
        primitives.len(),
        full.textures_delta.set.len()
    );

    for (id, deltas) in &full.textures_delta.set {
        for delta in deltas {
            renderer.update_texture(&device, &queue, *id, delta);
        }
    }
    // Applied, so cleared — before anything else can return or panic, because egui asserts
    // on drop that none are outstanding.
    full.textures_delta.clear();

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("frame"),
    });
    let user = renderer.update_buffers(&device, &queue, &mut encoder, &primitives, &screen);

    let clear = clear.to_array();
    {
        // **`forget_lifetime` is required, not a shortcut.** `Renderer::render` takes
        // `&mut RenderPass<'static>` — a pass whose descriptor borrowed nothing that could
        // outlive it — while `begin_render_pass` ties the pass to the `&mut encoder`. A
        // colour attachment necessarily borrows the texture view, so there is no spelling
        // that is 'static, and wgpu ships `forget_lifetime` for precisely this case. The
        // encoder is still alive here and is finished after the block; the escape is about
        // the borrow checker's view of the pass, not about the device's view of the buffer.
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("epsilon frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    // egui draws an opaque full-screen "background" mesh on the first frame, so
                    // the clear colour shows only where the window has genuinely nothing. That
                    // is the same colour `eframe` clears with (`app.clear_color(&visuals)`),
                    // and passing the app's own rather than a constant is what makes a blank
                    // area distinguishable from a missing one.
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: clear[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &primitives, &screen);
    }

    read_back(&device, &queue, encoder, &texture, width, height, user).await
}

/// Copy the rendered texture into a mappable buffer and read it back as RGBA8.
///
/// The encoder arrives **by value**: `CommandEncoder::finish` consumes it, and the copy
/// has to be encoded into the same encoder that drew the frame. Passing `&mut` and
/// calling `finish` on it is the borrow error this signature exists to prevent.
async fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    user: Vec<wgpu::CommandBuffer>,
) -> Result<Vec<u8>, String> {
    // **`copy_texture_to_buffer` requires a 256-byte-aligned row.** A 1440-wide RGBA8 row is
    // 5760 bytes, which is already aligned; a width that is not would silently corrupt
    // every row after the first if the padding were ignored, so the unpad step below is
    // not optional tidiness.
    const ALIGN: usize = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let unpadded = width as usize * 4;
    let padded = unpadded.div_ceil(ALIGN) * ALIGN;

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(user.into_iter().chain(Some(encoder.finish())));
    // **`map_async` is callback-based, not a future.** Calling it and reading the buffer
    // straight away reads a buffer the copy has not landed in yet, and returns zeroes —
    // which is a blank image and no error at all. `device.poll(Wait)` is what drives the
    // submission and the map callback to completion, and the buffer is only touched after
    // it returns. This is the one line standing between a real frame and a plausible-looking
    // grey rectangle.
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| {
        if let Err(e) = r {
            eprintln!("frame_raster: map failed: {e:?}");
        }
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("waiting for the readback: {e}"))?;
    let view = slice
        .get_mapped_range()
        .map_err(|e| format!("map range: {e}"))?;

    // BGRA is what a DX12 surface hands over; this target is RGBA, so no swizzle is needed
    // today. **Asserting it rather than assuming** keeps the next format change from
    // silently swapping the red and blue channels of every image this file produces — a
    // channel swap in a PNG is not a compile error and not a test failure, it is a picture
    // nobody looks at carefully.
    debug_assert!(
        matches!(
            texture.format(),
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
        ),
        "read_back assumes RGBA channel order, got {:?}",
        texture.format()
    );

    let mut out = Vec::with_capacity(unpadded * height as usize);
    for row in 0..height as usize {
        let start = row * padded;
        out.extend_from_slice(&view[start..start + unpadded]);
    }
    drop(view);
    buffer.unmap();
    Ok(out)
}

/// Write RGBA8 pixels to `path` as a PNG, creating parent directories.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let file =
        std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("png header: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("png data: {e}"))?;
    Ok(())
}
