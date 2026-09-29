//! The puppet's smooth skin drawn on the GPU (ADDITION, not in VellumFE):
//! `gs_puppet::Posed::skin_mesh` rasterised and shaded by `skin_gpu.wgsl`
//! straight into a v2-pass layer's atlas slot -- the same albedo, lit base
//! and geometry `field_gpu_stage::skin_image` makes from the CPU skin, with
//! no pixels crossing back. The doll's, behind its *GPU skin* toggle
//! (`doll_view`); a figure the mesh does not cover (unpainted, a status
//! effect) is drawn on the CPU.

use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;

/// One figure to draw into the atlas: its skin or its splat cloud, how its
/// lit base is lit, and metres per foot.
#[derive(Debug)]
pub struct SkinJob {
    pub figure: Figure,
    pub lit: SkinLit,
    pub m_per_ft: f32,
}

/// What a job draws.
#[derive(Debug, Clone)]
pub enum Figure {
    /// The smooth skin: its mesh, and its paint (None = unpainted).
    Skin { mesh: Arc<gs_puppet::SkinMesh>, texture: Option<Arc<gs_puppet::Texture>> },
    /// A splat cloud, projected and sorted (`gs_puppet::splat::prepare`).
    Splats(Arc<gs_puppet::splat::SplatFrame>),
}

impl Figure {
    /// The image's size.
    pub fn size(&self) -> (usize, usize) {
        match self {
            Figure::Skin { mesh, .. } => (mesh.w, mesh.h),
            Figure::Splats(f) => (f.w, f.h),
        }
    }
}

/// How a GPU skin's lit base -- what shows where no lamp reaches -- is lit.
#[derive(Debug, Clone)]
pub enum SkinLit {
    /// The doll's: its colours dimmed to the sky's floor (linear).
    Floor([f32; 3]),
    /// The stage's: relit by the scene and the time of day
    /// (`relight_gpu`), as `ResolvedLight::light_image` lights a CPU skin;
    /// with a sun (toward it in the SKIN's frame), first shaded by it
    /// (`SkinSurface::scene_lit`) and then given the time of day alone.
    Scene { plan: crate::relight_gpu::Plan, sun: Option<gs_puppet::SceneLight> },
}

impl SkinJob {
    /// The figure's depth span, metres from its feet, as
    /// `field_gpu::depth_span` finds it in a CPU skin's pixels: its surface
    /// points' depth along the view (the layer's geo depth).
    pub fn depth_span(&self) -> [f32; 2] {
        let (back, points): (_, Box<dyn Iterator<Item = [f32; 3]>>) = match &self.figure {
            Figure::Skin { mesh, .. } => (mesh.to_cam, Box::new(mesh.corners.iter().map(|c| c.pos))),
            Figure::Splats(f) => (f.to_cam, Box::new(f.flats.iter().map(|s| s.pos))),
        };
        let (lo, hi) = points.fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            let d = -(p[0] * back[0] + p[1] * back[1] + p[2] * back[2]) * self.m_per_ft;
            (lo.min(d), hi.max(d))
        });
        [lo, hi]
    }
}

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Bytes per splat (`Inst` in splat_gpu.wgsl).
const SPLAT_BYTES: u64 = 80;

/// Floats in the uniform block (`U` in the shader).
const UNIFORM_FLOATS: usize = 84;
/// The threads' spans (`fs_threads`): one texel each.
const SPANS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Bytes per mesh corner (`VIn`).
const CORNER_BYTES: u64 = 80;

/// The pipelines, and the sheets last uploaded (a figure's paint and marks
/// change far less often than it moves).
pub(crate) struct SkinGpu {
    raster: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    smooth: wgpu::RenderPipeline,
    threads: wgpu::RenderPipeline,
    finish: wgpu::RenderPipeline,
    compose: wgpu::RenderPipeline,
    compose_layout: wgpu::BindGroupLayout,
    splat_front: wgpu::RenderPipeline,
    splat_blend: wgpu::RenderPipeline,
    splat_final: wgpu::RenderPipeline,
    splat_layout: wgpu::BindGroupLayout,
    relight: crate::relight_gpu::RelightGpu,
    layout0: wgpu::BindGroupLayout,
    layout1: wgpu::BindGroupLayout,
    sheets: Mutex<Sheets>,
}

#[derive(Default)]
struct Sheets {
    /// (paint rgba's address and length, paint, normal sheet)
    paint: Option<((usize, usize), wgpu::TextureView, wgpu::TextureView)>,
    /// (the marks sheet's address, b+k, glow)
    marks: Option<(usize, wgpu::TextureView, wgpu::TextureView)>,
    /// 1 x 1 stand-ins: (rgba, r, a thread span)
    blank: Option<(wgpu::TextureView, wgpu::TextureView, wgpu::TextureView)>,
}

fn texture_entry(binding: u32, stage: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn make_texture(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn upload_rgba(device: &wgpu::Device, queue: &wgpu::Queue, w: u32, h: u32, format: wgpu::TextureFormat, bytes: &[u8], bpp: u32, label: &str) -> wgpu::TextureView {
    let tex = make_texture(device, w, h, format, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, label);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpp * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

fn storage(device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8], label: &str) -> wgpu::Buffer {
    // a storage binding may not be empty
    let size = (data.len() as u64).max(16).next_multiple_of(4);
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    if !data.is_empty() {
        queue.write_buffer(&buf, 0, data);
    }
    buf
}

fn attach(view: &wgpu::TextureView, clear: wgpu::Color) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store } })
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

impl SkinGpu {
    pub(crate) fn new(device: &wgpu::Device) -> SkinGpu {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skin_gpu"),
            source: wgpu::ShaderSource::Wgsl(include_str!("skin_gpu.wgsl").into()),
        });
        let frag = wgpu::ShaderStages::FRAGMENT;
        let layout0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin_gpu figure"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage_entry(1),
                storage_entry(2),
                storage_entry(3),
                storage_entry(4),
                texture_entry(5, frag),
                texture_entry(6, frag),
                texture_entry(7, frag),
                texture_entry(8, frag),
                storage_entry(9),
                storage_entry(10),
            ],
        });
        let layout1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skin_gpu buffers"),
            entries: &[texture_entry(0, frag), texture_entry(1, frag), texture_entry(2, frag), texture_entry(3, frag), texture_entry(4, frag)],
        });
        let raster_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("skin_gpu raster"), bind_group_layouts: &[Some(&layout0)], immediate_size: 0 });
        let full_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("skin_gpu passes"), bind_group_layouts: &[Some(&layout0), Some(&layout1)], immediate_size: 0 });
        let target = |format: wgpu::TextureFormat| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 8, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 12, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 16, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 28, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 32, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 44, shader_location: 6 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 56, shader_location: 7 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 72, shader_location: 8 },
        ];
        let raster = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skin_gpu raster"),
            layout: Some(&raster_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_raster"),
                buffers: &[wgpu::VertexBufferLayout { array_stride: CORNER_BYTES, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_raster"),
                targets: &[target(COLOR), target(COLOR), target(COLOR), target(COLOR)],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let full = |entry: &str, targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&full_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(entry), targets, compilation_options: wgpu::PipelineCompilationOptions::default() }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let resolve = full("fs_resolve", &[target(COLOR), target(COLOR), target(COLOR), target(COLOR)]);
        let smooth = full("fs_smooth", &[target(COLOR), target(COLOR)]);
        let threads = full("fs_threads", &[target(SPANS)]);
        let finish = full(
            "fs_final",
            &[target(wgpu::TextureFormat::Rgba8Unorm), target(wgpu::TextureFormat::Rgba8Unorm), target(wgpu::TextureFormat::Rgba16Float), target(wgpu::TextureFormat::Rgba8Unorm)],
        );
        let compose_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("skin_gpu compose"), entries: &[texture_entry(0, frag), texture_entry(1, frag)] });
        let compose_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("skin_gpu compose"), bind_group_layouts: &[Some(&compose_layout)], immediate_size: 0 });
        let compose = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skin_gpu compose"),
            layout: Some(&compose_pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_compose"), targets: &[target(wgpu::TextureFormat::Rgba8Unorm)], compilation_options: wgpu::PipelineCompilationOptions::default() }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        // ---- the splats (splat_gpu.wgsl)
        let splat_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("splat_gpu"), source: wgpu::ShaderSource::Wgsl(include_str!("splat_gpu.wgsl").into()) });
        let splat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splat_gpu"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                texture_entry(1, frag),
                texture_entry(2, frag),
                texture_entry(3, frag),
            ],
        });
        let splat_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("splat_gpu"), bind_group_layouts: &[Some(&splat_layout)], immediate_size: 0 });
        let inst_attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 8, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 32, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 36, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 48, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 52, shader_location: 6 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 64, shader_location: 7 },
        ];
        let splat_pipe = |entry: &str, vs: &str, targets: &[Option<wgpu::ColorTargetState>], depth: bool, instanced: bool| {
            let inst = [wgpu::VertexBufferLayout { array_stride: SPLAT_BYTES, step_mode: wgpu::VertexStepMode::Instance, attributes: &inst_attrs }];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&splat_pl),
                vertex: wgpu::VertexState { module: &splat_shader, entry_point: Some(vs), buffers: if instanced { &inst } else { &[] }, compilation_options: wgpu::PipelineCompilationOptions::default() },
                fragment: Some(wgpu::FragmentState { module: &splat_shader, entry_point: Some(entry), targets, compilation_options: wgpu::PipelineCompilationOptions::default() }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth.then_some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // back to front, premultiplied "over"
        let over = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let splat_front = splat_pipe("fs_front", "vs_splat", &[target(COLOR), target(COLOR)], true, true);
        let splat_blend = splat_pipe("fs_blend", "vs_splat", &[Some(wgpu::ColorTargetState { format: COLOR, blend: Some(over), write_mask: wgpu::ColorWrites::ALL })], false, true);
        let splat_final = splat_pipe(
            "fs_final",
            "vs_full",
            &[target(wgpu::TextureFormat::Rgba8Unorm), target(wgpu::TextureFormat::Rgba8Unorm), target(wgpu::TextureFormat::Rgba16Float), target(wgpu::TextureFormat::Rgba8Unorm)],
            false,
            false,
        );
        SkinGpu {
            raster,
            resolve,
            smooth,
            threads,
            finish,
            compose,
            compose_layout,
            splat_front,
            splat_blend,
            splat_final,
            splat_layout,
            relight: crate::relight_gpu::RelightGpu::new(device),
            layout0,
            layout1,
            sheets: Mutex::new(Sheets::default()),
        }
    }

    /// Draw `job` into the atlas `albedo` / `lit` / `geo` with its image's
    /// top-left at `at` and `pad` cleared pixels round it (the slot's
    /// gutter), recorded on `encoder`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        job: &SkinJob,
        atlas: [&wgpu::Texture; 3],
        at: [u32; 2],
        pad: u32,
    ) {
        let (m, texture) = match &job.figure {
            Figure::Skin { mesh, texture } => (mesh, texture),
            Figure::Splats(frame) => return self.draw_splats(device, queue, encoder, job, frame, atlas, at, pad),
        };
        let (w, h) = (m.w as u32, m.h as u32);
        let ss = gs_puppet::SKIN_SS as u32;
        if w == 0 || h == 0 || m.corners.is_empty() {
            return;
        }
        let mut sheets = self.sheets.lock().unwrap_or_else(|e| e.into_inner());
        if sheets.blank.is_none() {
            let rgba = upload_rgba(device, queue, 1, 1, wgpu::TextureFormat::Rgba8Unorm, &[0, 0, 0, 255], 4, "skin_gpu blank");
            let r = upload_rgba(device, queue, 1, 1, wgpu::TextureFormat::R8Unorm, &[0; 4][..1], 1, "skin_gpu blank r");
            let span = upload_rgba(device, queue, 1, 1, SPANS, &[0; 16], 16, "skin_gpu blank span");
            sheets.blank = Some((rgba, r, span));
        }
        // the paint and its normal sheet, uploaded once per texture
        if let Some(tex) = &texture {
            let paint_key = (tex.rgba.as_ptr() as usize, tex.rgba.len());
            if sheets.paint.as_ref().is_none_or(|(k, _, _)| *k != paint_key) {
            let (pw, ph) = (tex.layout.w as u32, tex.layout.h as u32);
            let paint = upload_rgba(device, queue, pw, ph, wgpu::TextureFormat::Rgba8Unorm, &tex.rgba, 4, "skin_gpu paint");
            let normal = match &tex.normal {
                Some(n) if n.len() == tex.rgba.len() => upload_rgba(device, queue, pw, ph, wgpu::TextureFormat::Rgba8Unorm, n, 4, "skin_gpu normals"),
                _ => upload_rgba(device, queue, 1, 1, wgpu::TextureFormat::Rgba8Unorm, &[0; 4], 4, "skin_gpu no normals"),
            };
            sheets.paint = Some((paint_key, paint, normal));
            }
        }
        if let Some((sheet, _)) = &m.marks {
            let key = Arc::as_ptr(sheet) as usize;
            if sheets.marks.as_ref().is_none_or(|(k, _, _)| *k != key) {
                let [sw, sh] = sheet.size();
                let (bk, glow) = sheet.texels();
                let bytes: Vec<u8> = bk.iter().flatten().copied().collect();
                let bk = upload_rgba(device, queue, sw as u32, sh as u32, wgpu::TextureFormat::Rgba8Unorm, &bytes, 4, "skin_gpu marks");
                // glow and wetness, side by side
                let gw: Vec<u8> = glow.iter().zip(sheet.wet()).flat_map(|(g, w)| [*g, *w]).collect();
                let glow = upload_rgba(device, queue, sw as u32, sh as u32, wgpu::TextureFormat::Rg8Unorm, &gw, 2, "skin_gpu glow");
                sheets.marks = Some((key, bk, glow));
            }
        }
        let blank = sheets.blank.as_ref().expect("made above");
        let has_normal = texture.as_ref().is_some_and(|t| t.normal.as_ref().is_some_and(|n| n.len() == t.rgba.len()));
        let (paint, normal) = match (&texture, &sheets.paint) {
            (Some(_), Some((_, p, n))) => (p, n),
            _ => (&blank.0, &blank.0),
        };
        let paint_size = texture.as_ref().map_or([1, 1], |t| [t.layout.w, t.layout.h]);
        let (marks_bk, marks_glow, marks_size, marks_per_ft) = match (&m.marks, &sheets.marks) {
            (Some((sheet, _)), Some((_, bk, glow))) => (bk, glow, sheet.size(), sheet.per_ft()),
            _ => (&blank.0, &blank.1, [1, 1], 1.0),
        };

        // ---- the figure's own data
        let s = &m.shading;
        let v3 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
        let mut u = Vec::with_capacity(UNIFORM_FLOATS);
        u.extend([w as f32, h as f32, ss as f32, pad as f32]);
        u.extend([m.near, m.range, m.blend_tol, m.contour_tol]);
        u.extend(v3(m.light, m.outline_px));
        u.extend(v3(m.to_cam, if m.boss { 1.0 } else { 0.0 }));
        u.extend(v3(m.line.map(|c| c / 255.0), m.smooth_radius as f32));
        u.extend([s.light_above, s.dark_below, s.band_soft, s.lit_tone]);
        u.extend([s.dark_tone, s.lambert, s.relief_gain, s.normal_map_strength]);
        u.extend([s.depth_shade, s.joined_blend, s.boss_extra_px, job.m_per_ft]);
        u.extend(v3(s.boss_rgb.map(|c| c / 255.0), m.segments as f32));
        let floor = match &job.lit {
            SkinLit::Floor(f) => *f,
            SkinLit::Scene { .. } => [1.0; 3],
        };
        u.extend(v3(floor, if has_normal { 1.0 } else { 0.0 }));
        let flags = |x: f32, y: f32| [if m.marks.is_some() { 1.0 } else { 0.0 }, x, y, marks_per_ft];
        u.extend(flags(0.0, 0.0));
        u.extend([paint_size[0] as f32, paint_size[1] as f32, marks_size[0] as f32, marks_size[1] as f32]);
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        u.extend([flag(texture.is_some()), flag(m.effects.frozen), flag(m.effects.webbed), m.strand_ft]);
        u.extend([m.thread_half, m.threads.len() as f32, flag(m.smooth_colours), m.eyes.len() as f32]);
        u.extend(v3(s.web_rgb.map(|c| c / 255.0), s.web_spacing_ft));
        u.extend(v3(s.frost_rgb.map(|c| c / 255.0), 0.0));
        u.extend(v3(s.eye_rgb.map(|c| c / 255.0), 0.0));
        let sun = match &job.lit {
            SkinLit::Scene { sun: Some(sun), .. } => Some(sun),
            _ => None,
        };
        let s2l = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let toward = sun.map_or([0.0, 1.0, 0.0], |l| {
            let n = (l.toward[0].powi(2) + l.toward[1].powi(2) + l.toward[2].powi(2)).sqrt();
            if n > 1e-6 { l.toward.map(|v| v / n) } else { [0.0, 1.0, 0.0] }
        });
        u.extend(v3(toward, flag(sun.is_some())));
        u.extend(v3(sun.map_or([0.0; 3], |l| l.rgb.map(|c| s2l(c) * l.strength)), 0.0));
        u.extend(v3(sun.map_or([0.0; 3], |l| l.fill_rgb.map(|c| s2l(c) * l.fill)), 0.0));
        u.extend([s.scene_wrap, s.scene_rim, s.scene_rim_power, s.fill_ground]);
        debug_assert_eq!(u.len(), UNIFORM_FLOATS);
        let uniform = |u: &[f32], label: &str| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (UNIFORM_FLOATS * 4) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            queue.write_buffer(&buf, 0, &floats(u));
            buf
        };
        let marked: Vec<bool> = m.marks.as_ref().map(|(_, segs)| segs.clone()).unwrap_or_default();
        let mut seg_data = Vec::with_capacity(m.segments * 16);
        for (i, rot) in m.seg_rot.iter().enumerate() {
            for col in rot {
                seg_data.extend(v3(*col, 0.0));
            }
            seg_data.extend([flag(marked.get(i).copied().unwrap_or(false)), flag(m.garment.get(i).copied().unwrap_or(false)), 0.0, 0.0]);
        }
        let joined: Vec<u8> = m.joined.iter().flat_map(|j| (*j as u32).to_le_bytes()).collect();
        let surface_layout = m.surface_layout.clone().unwrap_or_else(|| vec![0.0, 1.0]);
        let paint_layout = if m.paint_layout.is_empty() { vec![0.0, 1.0] } else { m.paint_layout.clone() };
        let eyes: Vec<f32> = m.eyes.iter().flatten().copied().collect();
        let threads: Vec<f32> = m.threads.iter().flat_map(|t| [t[0], t[1], t[2], t[3], t[4], 0.0, 0.0, 0.0]).collect();
        let buffers = [
            storage(device, queue, &floats(&paint_layout), "skin_gpu paint layout"),
            storage(device, queue, &floats(&surface_layout), "skin_gpu surface layout"),
            storage(device, queue, &floats(&seg_data), "skin_gpu segments"),
            storage(device, queue, &joined, "skin_gpu joined"),
            storage(device, queue, &floats(&eyes), "skin_gpu eyes"),
            storage(device, queue, &floats(&threads), "skin_gpu threads"),
        ];
        let group0 = |uniform: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("skin_gpu figure"),
                layout: &self.layout0,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: buffers[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: buffers[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: buffers[2].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: buffers[3].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(paint) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(normal) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(marks_bk) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(marks_glow) },
                    wgpu::BindGroupEntry { binding: 9, resource: buffers[4].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 10, resource: buffers[5].as_entire_binding() },
                ],
            })
        };
        let u_main = uniform(&u, "skin_gpu uniforms");
        let g_main = group0(&u_main);

        // ---- the mesh
        let mut corners: Vec<u8> = Vec::with_capacity(m.corners.len() * CORNER_BYTES as usize);
        for c in &m.corners {
            let mut put = |v: f32| corners.extend(v.to_le_bytes());
            put(c.p[0]);
            put(c.p[1]);
            put(c.z);
            corners.extend(c.seg.to_le_bytes());
            let mut put = |v: f32| corners.extend(v.to_le_bytes());
            for v in c.n {
                put(v);
            }
            put(if c.addressed { 1.0 } else { 0.0 });
            for v in c.pos.into_iter().chain(c.col).chain(c.wound).chain(c.addr) {
                put(v);
            }
        }
        let vertices = device.create_buffer(&wgpu::BufferDescriptor { label: Some("skin_gpu mesh"), size: corners.len() as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&vertices, 0, &corners);

        // ---- the passes' targets
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let views = |tw: u32, th: u32, label: &str| -> [wgpu::TextureView; 4] {
            std::array::from_fn(|_| make_texture(device, tw, th, COLOR, rt, label).create_view(&wgpu::TextureViewDescriptor::default()))
        };
        let samples = views(w * ss, h * ss, "skin_gpu samples");
        let depth = make_texture(device, w * ss, h * ss, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT, "skin_gpu depth").create_view(&wgpu::TextureViewDescriptor::default());
        let pixels = views(w, h, "skin_gpu pixels");
        let smoothed: [[wgpu::TextureView; 2]; 2] =
            std::array::from_fn(|_| std::array::from_fn(|_| make_texture(device, w, h, COLOR, rt, "skin_gpu smoothed").create_view(&wgpu::TextureViewDescriptor::default())));
        let spans_tex = (!m.threads.is_empty()).then(|| make_texture(device, m.threads.len() as u32, 1, SPANS, rt, "skin_gpu thread spans").create_view(&wgpu::TextureViewDescriptor::default()));
        let (ow, oh) = (w + 2 * pad, h + 2 * pad);
        let out_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let out_usage = out_usage | wgpu::TextureUsages::TEXTURE_BINDING;
        let outs = [
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "skin_gpu albedo"),
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "skin_gpu lit"),
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba16Float, out_usage, "skin_gpu geo"),
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "skin_gpu rgba"),
        ];
        let out_views: Vec<wgpu::TextureView> = outs.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect();
        let group1 = |col: &wgpu::TextureView, n: &wgpu::TextureView, of: &[wgpu::TextureView; 4], spans: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("skin_gpu buffers"),
                layout: &self.layout1,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(col) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(n) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&of[2]) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&of[3]) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(spans) },
                ],
            })
        };
        let no_spans = &blank.2;

        let none = wgpu::Color::TRANSPARENT;
        let far = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 60000.0 };
        let no_seg = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: -1.0 };

        // raster
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("skin_gpu raster"),
                color_attachments: &[attach(&samples[0], none), attach(&samples[1], none), attach(&samples[2], no_seg), attach(&samples[3], far)],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.raster);
            pass.set_bind_group(0, &g_main, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..m.corners.len() as u32, 0..1);
        }
        // resolve
        {
            let g1 = group1(&samples[0], &samples[1], &samples, no_spans);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("skin_gpu resolve"),
                color_attachments: &[attach(&pixels[0], none), attach(&pixels[1], none), attach(&pixels[2], no_seg), attach(&pixels[3], far)],
                ..Default::default()
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &g_main, &[]);
            pass.set_bind_group(1, &g1, &[]);
            pass.draw(0..3, 0..1);
        }
        // smooth, x then y
        let (mut normals, mut colours) = (&pixels[1], &pixels[0]);
        if m.smooth_radius > 0 {
            for (k, (dx, dy)) in [(1.0, 0.0), (0.0, 1.0)].into_iter().enumerate() {
                let mut ua = u.clone();
                let f = 40;
                ua[f + 1] = dx;
                ua[f + 2] = dy;
                let ub = uniform(&ua, "skin_gpu smooth uniforms");
                let g0 = group0(&ub);
                let g1 = group1(colours, normals, &pixels, no_spans);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skin_gpu smooth"),
                    color_attachments: &[attach(&smoothed[k][0], none), attach(&smoothed[k][1], none)],
                    ..Default::default()
                });
                pass.set_pipeline(&self.smooth);
                pass.set_bind_group(0, &g0, &[]);
                pass.set_bind_group(1, &g1, &[]);
                pass.draw(0..3, 0..1);
                drop(pass);
                normals = &smoothed[k][0];
                colours = &smoothed[k][1];
            }
        }
        // a webbed figure's threads: where each first and last meets the body
        if let Some(spans) = &spans_tex {
            let g1 = group1(colours, normals, &pixels, no_spans);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("skin_gpu threads"),
                color_attachments: &[attach(spans, none)],
                ..Default::default()
            });
            pass.set_pipeline(&self.threads);
            pass.set_bind_group(0, &g_main, &[]);
            pass.set_bind_group(1, &g1, &[]);
            pass.draw(0..3, 0..1);
        }
        // shade, contours, the layer
        {
            let g1 = group1(colours, normals, &pixels, spans_tex.as_ref().unwrap_or(no_spans));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("skin_gpu final"),
                color_attachments: &[attach(&out_views[0], none), attach(&out_views[1], none), attach(&out_views[2], none), attach(&out_views[3], none)],
                ..Default::default()
            });
            pass.set_pipeline(&self.finish);
            pass.set_bind_group(0, &g_main, &[]);
            pass.set_bind_group(1, &g1, &[]);
            pass.draw(0..3, 0..1);
        }
        self.finish(device, queue, encoder, &job.lit, &outs, ow, oh, atlas, at, pad);
    }

    /// A splat cloud into the atlas, as `draw` a skin: front, blend, final,
    /// then `finish`.
    #[allow(clippy::too_many_arguments)]
    fn draw_splats(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        job: &SkinJob,
        frame: &gs_puppet::splat::SplatFrame,
        atlas: [&wgpu::Texture; 3],
        at: [u32; 2],
        pad: u32,
    ) {
        let (w, h) = (frame.w as u32, frame.h as u32);
        if w == 0 || h == 0 || frame.flats.is_empty() {
            return;
        }
        let blank = {
            let mut sheets = self.sheets.lock().unwrap_or_else(|e| e.into_inner());
            if sheets.blank.is_none() {
                let rgba = upload_rgba(device, queue, 1, 1, wgpu::TextureFormat::Rgba8Unorm, &[0, 0, 0, 255], 4, "skin_gpu blank");
                let r = upload_rgba(device, queue, 1, 1, wgpu::TextureFormat::R8Unorm, &[0; 4][..1], 1, "skin_gpu blank r");
                let span = upload_rgba(device, queue, 1, 1, SPANS, &[0; 16], 16, "skin_gpu blank span");
                sheets.blank = Some((rgba, r, span));
            }
            sheets.blank.as_ref().expect("made above").0.clone()
        };
        let (near, far) = frame.flats.iter().fold((f32::MAX, f32::MIN), |(n, f), s| (n.min(s.z), f.max(s.z)));
        let floor = match &job.lit {
            SkinLit::Floor(f) => *f,
            SkinLit::Scene { .. } => [1.0; 3],
        };
        let uniform = |reach: f32| {
            let u: [f32; 16] = [
                w as f32, h as f32, pad as f32, job.m_per_ft,
                near, (far - near).max(1e-3), gs_puppet::splat::SURFACE_BAND, reach,
                frame.to_cam[0], frame.to_cam[1], frame.to_cam[2], 0.0,
                floor[0], floor[1], floor[2], 0.0,
            ];
            let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("splat_gpu uniforms"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            queue.write_buffer(&buf, 0, &floats(&u));
            buf
        };
        let mut inst: Vec<u8> = Vec::with_capacity(frame.flats.len() * SPLAT_BYTES as usize);
        for s in &frame.flats {
            let v: [f32; 20] = [
                s.c[0], s.c[1], s.inv[0], s.inv[1], s.inv[2], s.inv[3], s.r[0], s.r[1], s.z, s.rgb[0], s.rgb[1], s.rgb[2], s.alpha, s.pos[0], s.pos[1], s.pos[2], s.n[0], s.n[1], s.n[2], 0.0,
            ];
            inst.extend(floats(&v));
        }
        let instances = device.create_buffer(&wgpu::BufferDescriptor { label: Some("splat_gpu splats"), size: inst.len() as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        queue.write_buffer(&instances, 0, &inst);
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = |t: wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let front_n = view(make_texture(device, w, h, COLOR, rt, "splat_gpu front normal"));
        let front_pos = view(make_texture(device, w, h, COLOR, rt, "splat_gpu front point"));
        let acc = view(make_texture(device, w, h, COLOR, rt, "splat_gpu blend"));
        let depth = view(make_texture(device, w, h, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT, "splat_gpu depth"));
        let group = |u: &wgpu::Buffer, a: &wgpu::TextureView, b: &wgpu::TextureView, c: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("splat_gpu"),
                layout: &self.splat_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(b) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(c) },
                ],
            })
        };
        let none = wgpu::Color::TRANSPARENT;
        let far_c = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 60000.0 };
        let n = frame.flats.len() as u32;
        // the front surface: each core
        {
            let u = uniform(0.5);
            let g = group(&u, &blank, &blank, &blank);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("splat_gpu front"),
                color_attachments: &[attach(&front_n, none), attach(&front_pos, far_c)],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.splat_front);
            pass.set_bind_group(0, &g, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..6, 0..n);
        }
        // the splats, back to front
        {
            let u = uniform(1.0);
            let g = group(&u, &front_n, &front_pos, &blank);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("splat_gpu blend"), color_attachments: &[attach(&acc, none)], ..Default::default() });
            pass.set_pipeline(&self.splat_blend);
            pass.set_bind_group(0, &g, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            pass.draw(0..6, 0..n);
        }
        // resolved into the layer
        let (ow, oh) = (w + 2 * pad, h + 2 * pad);
        let out_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
        let outs = [
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "splat_gpu albedo"),
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "splat_gpu lit"),
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba16Float, out_usage, "splat_gpu geo"),
            make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "splat_gpu rgba"),
        ];
        {
            let out_views: Vec<wgpu::TextureView> = outs.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect();
            let u = uniform(1.0);
            let g = group(&u, &front_n, &front_pos, &acc);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("splat_gpu final"),
                color_attachments: &[attach(&out_views[0], none), attach(&out_views[1], none), attach(&out_views[2], none), attach(&out_views[3], none)],
                ..Default::default()
            });
            pass.set_pipeline(&self.splat_final);
            pass.set_bind_group(0, &g, &[]);
            pass.draw(0..3, 0..1);
        }
        self.finish(device, queue, encoder, &job.lit, &outs, ow, oh, atlas, at, pad);
    }

    /// A figure's four outputs -- albedo, lit, geo, and its plain colours --
    /// into the atlas at `at` (gutter `pad`): the lit base relit first when
    /// `lit` is the stage's.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        lit: &SkinLit,
        outs: &[wgpu::Texture; 4],
        ow: u32,
        oh: u32,
        atlas: [&wgpu::Texture; 3],
        at: [u32; 2],
        pad: u32,
    ) {
        let none = wgpu::Color::TRANSPARENT;
        let out_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
        let lit_view = outs[1].create_view(&wgpu::TextureViewDescriptor::default());
        // the stage's: the skin relit by the scene and the time of day, its
        // glow kept, as the lit base
        let relit_lit = match lit {
            SkinLit::Floor(_) => None,
            SkinLit::Scene { plan, .. } => {
                let relit = self.relight.run(device, queue, encoder, &outs[3], ow, oh, plan);
                let composed = make_texture(device, ow, oh, wgpu::TextureFormat::Rgba8Unorm, out_usage, "skin_gpu relit lit");
                let view = composed.create_view(&wgpu::TextureViewDescriptor::default());
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("skin_gpu compose"),
                    layout: &self.compose_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&relit.create_view(&wgpu::TextureViewDescriptor::default())) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lit_view) },
                    ],
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("skin_gpu compose"), color_attachments: &[attach(&view, none)], ..Default::default() });
                pass.set_pipeline(&self.compose);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
                drop(pass);
                Some(composed)
            }
        };
        // into the atlas, gutter and all
        let origin = wgpu::Origin3d { x: at[0].saturating_sub(pad), y: at[1].saturating_sub(pad), z: 0 };
        let lit_src = relit_lit.as_ref().unwrap_or(&outs[1]);
        for (src, dst) in [&outs[0], lit_src, &outs[2]].into_iter().zip(atlas) {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: dst, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: ow, height: oh, depth_or_array_layers: 1 },
            );
        }
    }
}

/// Skins waiting to be drawn, by the image key their (pixel-less)
/// `LitImage` carries: the field pass draws an image with no pixels from
/// its job here. Newest last; a few kept.
fn jobs() -> &'static Mutex<Vec<(u64, Arc<SkinJob>)>> {
    static JOBS: std::sync::OnceLock<Mutex<Vec<(u64, Arc<SkinJob>)>>> = std::sync::OnceLock::new();
    JOBS.get_or_init(Default::default)
}

/// Enough for a crowded stage and a doll.
const JOBS_KEPT: usize = 96;

/// Register `job` to be drawn for the image keyed `key` (one with no
/// pixels of its own: `LitImage::gpu_only`).
pub fn submit(key: u64, job: Arc<SkinJob>) {
    let mut list = jobs().lock().unwrap_or_else(|e| e.into_inner());
    list.retain(|(k, _)| *k != key);
    if list.len() >= JOBS_KEPT {
        list.remove(0);
    }
    list.push((key, job));
}

/// The job registered for image `key`.
pub fn job(key: u64) -> Option<Arc<SkinJob>> {
    jobs().lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key).map(|(_, j)| j.clone())
}

#[cfg(test)]
mod tests {
    /// The shaders parse and validate, as wgpu would load them -- without
    /// a GPU. (WGSL is otherwise only checked when a device compiles it.)
    #[test]
    fn the_shaders_validate() {
        for (name, src) in [("skin_gpu.wgsl", include_str!("skin_gpu.wgsl")), ("field_gpu.wgsl", include_str!("field_gpu.wgsl")), ("relight_gpu.wgsl", include_str!("relight_gpu.wgsl")), ("splat_gpu.wgsl", include_str!("splat_gpu.wgsl"))] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}
