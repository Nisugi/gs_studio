//! vellum_light's relight on the GPU (ADDITION, not in VellumFE): a skin
//! drawn by `skin_gpu` is lit by the scene and the time of day where it is,
//! as `creature_light::ResolvedLight::light_image` lights a CPU skin -- the
//! same kernel (`light_pixel`), the same frame (the 4th / 96th percentile
//! of OKLab L, from a 4096-bin histogram instead of a sort, and the content
//! rows) and the same byte-space cross-fade of keyframes. `examples/
//! relight_golden` checks it against vellum_light's golden cases.

use egui_wgpu::wgpu;
use vellum_light::{Derived, LightEnv};

const RGBA8: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Histogram bins, then count, top row, bottom row (`hist` in the shader).
const BINS: usize = 4096;

/// One light pass's parameters (`P` in the shader).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pass {
    pub key_tint: [f32; 3],
    pub ambient_tint: [f32; 3],
    pub bounce_tint: [f32; 3],
    pub fog_lab: [f32; 3],
    pub exposure: f32,
    pub white: f32,
    pub chroma: f32,
    pub strength: f32,
    pub bounce_strength: f32,
    pub night: f32,
    pub haze: f32,
}

impl Pass {
    /// `env` and `d` as a pass; None for the identity light (it changes
    /// nothing: the CPU skips it).
    pub fn of(env: &LightEnv, d: &Derived) -> Option<Pass> {
        (!d.identity).then_some(Pass {
            key_tint: d.key_tint,
            ambient_tint: d.ambient_tint,
            bounce_tint: d.bounce_tint,
            fog_lab: d.fog_lab,
            exposure: d.exposure,
            white: d.white,
            chroma: d.chroma,
            strength: env.strength,
            bounce_strength: env.bounce_strength,
            night: env.night,
            haze: env.haze,
        })
    }

    fn uniforms(&self) -> Vec<u8> {
        let v4 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
        let f: Vec<f32> = [v4(self.key_tint, 0.0), v4(self.ambient_tint, 0.0), v4(self.bounce_tint, 0.0), v4(self.fog_lab, 0.0), [self.exposure, self.white, self.chroma, self.strength], [self.bounce_strength, self.night, self.haze, 0.0]]
            .concat();
        f.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
}

/// What to do to an image: the scene's pass (None: skip it, as the time of
/// day alone on a sun-lit skin), then the time-of-day keyframes (None =
/// the identity, day) cross-faded by weight.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub scene: Option<Pass>,
    pub keyframes: Vec<(Option<Pass>, f32)>,
}

impl Plan {
    /// `creature_light`'s plan: `(scene, keyframes)` as `ResolvedLight`
    /// holds them.
    pub fn of(scene: Option<&(LightEnv, Derived)>, keyframes: &[((LightEnv, Derived), f32)]) -> Plan {
        Plan { scene: scene.and_then(|(e, d)| Pass::of(e, d)), keyframes: keyframes.iter().map(|((e, d), w)| (Pass::of(e, d), *w)).collect() }
    }
}

pub(crate) struct RelightGpu {
    stats: wgpu::ComputePipeline,
    frame: wgpu::ComputePipeline,
    light: wgpu::RenderPipeline,
    fade: wgpu::RenderPipeline,
    stats_layout: wgpu::BindGroupLayout,
    light_layout: wgpu::BindGroupLayout,
    fade_layout: wgpu::BindGroupLayout,
}

fn tex_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        count: None,
    }
}

fn buf_entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None }
}

impl RelightGpu {
    pub(crate) fn new(device: &wgpu::Device) -> RelightGpu {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("relight_gpu"), source: wgpu::ShaderSource::Wgsl(include_str!("relight_gpu.wgsl").into()) });
        let cs = wgpu::ShaderStages::COMPUTE;
        let fs = wgpu::ShaderStages::FRAGMENT;
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let stats_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("relight stats"),
            entries: &[tex_entry(0, cs), buf_entry(1, cs, rw), buf_entry(2, cs, rw)],
        });
        let light_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("relight light"),
            entries: &[buf_entry(0, fs, wgpu::BufferBindingType::Uniform), buf_entry(1, fs, wgpu::BufferBindingType::Storage { read_only: true }), tex_entry(2, fs)],
        });
        let fade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("relight fade"),
            entries: &[buf_entry(0, fs, wgpu::BufferBindingType::Uniform), tex_entry(1, fs), tex_entry(2, fs), tex_entry(3, fs)],
        });
        let compute = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&pl), module: &shader, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let render = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format: RGBA8, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        RelightGpu {
            stats: compute(&stats_layout, "cs_stats"),
            frame: compute(&stats_layout, "cs_frame"),
            light: render(&light_layout, "fs_light"),
            fade: render(&fade_layout, "fs_fade"),
            stats_layout,
            light_layout,
            fade_layout,
        }
    }

    fn target(device: &wgpu::Device, w: u32, h: u32, label: &str) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: RGBA8,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// `LightFrame::measure` of `src` (w x h): a buffer of l_lo, l_hi,
    /// y_top, y_bot.
    fn measure(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, src: &wgpu::TextureView, w: u32, h: u32) -> wgpu::Buffer {
        let mut init = vec![0u32; BINS + 3];
        init[BINS + 1] = u32::MAX;
        let hist = device.create_buffer(&wgpu::BufferDescriptor { label: Some("relight histogram"), size: (init.len() * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&hist, 0, &init.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        let frame = device.create_buffer(&wgpu::BufferDescriptor { label: Some("relight frame"), size: 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("relight stats"),
            layout: &self.stats_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 1, resource: hist.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: frame.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("relight stats"), timestamp_writes: None });
        pass.set_bind_group(0, &group, &[]);
        pass.set_pipeline(&self.stats);
        pass.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);
        pass.set_pipeline(&self.frame);
        pass.dispatch_workgroups(1, 1, 1);
        drop(pass);
        frame
    }

    fn light_pass(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, src: &wgpu::TextureView, frame: &wgpu::Buffer, pass: &Pass, w: u32, h: u32) -> wgpu::Texture {
        let out = Self::target(device, w, h, "relight lit");
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("relight pass"), size: 96, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&uniform, 0, &pass.uniforms());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("relight light"),
            layout: &self.light_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: frame.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(src) },
            ],
        });
        let view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("relight light"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
        rp.set_pipeline(&self.light);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
        drop(rp);
        out
    }

    /// `src` (RGBA8, w x h, straight alpha) relit as `plan` says, into a
    /// new RGBA8 texture -- `bake_one` / `light_image` (or, with no scene
    /// pass, `light_image_timed`) on the GPU. Recorded on `encoder`.
    pub(crate) fn run(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, src: &wgpu::Texture, w: u32, h: u32, plan: &Plan) -> wgpu::Texture {
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());
        // the scene pass, measured on the image as it comes
        let lit = plan.scene.as_ref().map(|scene| {
            let frame = self.measure(device, queue, encoder, &src_view, w, h);
            self.light_pass(device, queue, encoder, &src_view, &frame, scene, w, h)
        });
        let lit_view = lit.as_ref().map_or_else(|| src_view.clone(), |t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        let weighted: Vec<&(Option<Pass>, f32)> = plan.keyframes.iter().filter(|(_, w)| *w > 0.0).collect();
        if weighted.is_empty() {
            return match lit {
                Some(t) => t,
                None => self.copy(device, encoder, src, w, h),
            };
        }
        // the keyframes, measured on the scene-lit bytes
        let frame = self.measure(device, queue, encoder, &lit_view, w, h);
        let passes: Vec<Option<wgpu::Texture>> = weighted.iter().map(|(p, _)| p.as_ref().map(|p| self.light_pass(device, queue, encoder, &lit_view, &frame, p, w, h))).collect();
        if let [only] = passes.as_slice() {
            // one keyframe: its pass alone, not faded (as `blend_keyframes`)
            return match only {
                Some(t) => self.copy(device, encoder, t, w, h),
                None => lit.unwrap_or_else(|| self.copy(device, encoder, src, w, h)),
            };
        }
        let total: f32 = weighted.iter().map(|(_, w)| *w).sum::<f32>().max(1e-6);
        let views: Vec<wgpu::TextureView> = passes.iter().map(|t| t.as_ref().map_or_else(|| lit_view.clone(), |t| t.create_view(&wgpu::TextureViewDescriptor::default()))).collect();
        let wt: Vec<f32> = weighted.iter().map(|(_, w)| *w / total).collect();
        let n = views.len().min(3);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("relight fade"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let u = [wt.first().copied().unwrap_or(0.0), wt.get(1).copied().unwrap_or(0.0), wt.get(2).copied().unwrap_or(0.0), n as f32];
        queue.write_buffer(&uniform, 0, &u.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        let pick = |i: usize| &views[i.min(n - 1)];
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("relight fade"),
            layout: &self.fade_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(pick(0)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(pick(1)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(pick(2)) },
            ],
        });
        let out = Self::target(device, w, h, "relight faded");
        let view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("relight fade"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            ..Default::default()
        });
        rp.set_pipeline(&self.fade);
        rp.set_bind_group(0, &group, &[]);
        rp.draw(0..3, 0..1);
        drop(rp);
        out
    }

    fn copy(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, src: &wgpu::Texture, w: u32, h: u32) -> wgpu::Texture {
        let out = Self::target(device, w, h, "relight copy");
        encoder.copy_texture_to_texture(src.as_image_copy(), out.as_image_copy(), wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        out
    }
}

/// Relight RGBA8 pixels on `device` and read them back: for checking the
/// port against vellum_light (examples/relight_golden), not for frames.
pub fn relight_pixels(device: &wgpu::Device, queue: &wgpu::Queue, rgba: &[u8], w: u32, h: u32, plan: &Plan) -> Vec<u8> {
    let gpu = RelightGpu::new(device);
    let src = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("relight input"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: RGBA8,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    queue.write_texture(src.as_image_copy(), rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("relight") });
    let out = gpu.run(device, queue, &mut encoder, &src, w, h, plan);
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("relight readback"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    encoder.copy_texture_to_buffer(
        out.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer.slice(..).get_mapped_range();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        out.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize * 4]);
    }
    out
}
