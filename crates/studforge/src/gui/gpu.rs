//! wgpu renderer for the 3D viewer, drawn inside egui's render pass through an
//! `egui_wgpu::Callback`.
//!
//! One vertex/index buffer per distinct part mesh (LDraw coordinates, LDU),
//! drawn instanced: each instance carries a 4×4 transform (LDraw part frame →
//! y-up world in studs), a colour and flags. LDraw edge lines are drawn the
//! same way, then the ground grid and selection outline as plain lines.

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use sf_core::Palette;
use sf_ldraw::{MAIN_COLOR, PartMesh};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

/// Multisampling used for the whole window (must match `NativeOptions`).
pub const MSAA: u32 = 4;
/// Depth format used for the whole window (`NativeOptions::depth_buffer = 32`).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub const FLAG_DIM: u32 = 1;
pub const FLAG_HIGHLIGHT: u32 = 2;
pub const FLAG_SELECTED: u32 = 4;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct MeshVertex {
    pos: [f32; 3],
    normal: [f32; 3],
    /// sRGB; alpha 255 = use this colour, 0 = use the instance colour.
    color: [u8; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct EdgeVertex {
    pos: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    /// Column-major 4×4 model matrix (LDraw part frame → world).
    pub model: [[f32; 4]; 4],
    /// sRGB colour.
    pub color: [u8; 4],
    pub flags: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    /// sRGB + alpha.
    pub color: [u8; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Globals {
    /// Column-major view-projection.
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    /// Direction toward the key light (world).
    pub key_dir: [f32; 4],
    pub fill_dir: [f32; 4],
    /// Linear accent colour.
    pub accent: [f32; 4],
    /// x: 1 if the target is sRGB (write linear), y: edge alpha, z: pulse 0..1.
    pub params: [f32; 4],
}

/// Everything the viewer wants drawn, rebuilt when the model, step or
/// selection changes (tracked by `generation`).
pub struct Scene {
    pub generation: u64,
    pub palette: Palette,
    /// (part id, mesh, instances).
    pub batches: Vec<(String, Arc<PartMesh>, Vec<Instance>)>,
    pub lines: Vec<LineVertex>,
    pub overlay_lines: Vec<LineVertex>,
    pub edges: bool,
}

pub struct ViewerCallback {
    pub scene: Arc<Scene>,
    pub globals: Globals,
}

struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    ebuf: Option<wgpu::Buffer>,
    edge_vertex_count: u32,
}

/// Per-batch instance range in the shared instance buffer.
struct Batch {
    part: String,
    range: std::ops::Range<u32>,
}

pub struct Renderer {
    mesh_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    meshes: HashMap<String, GpuMesh>,
    instance_buf: Option<wgpu::Buffer>,
    line_buf: Option<wgpu::Buffer>,
    line_count: u32,
    overlay_buf: Option<wgpu::Buffer>,
    overlay_count: u32,
    batches: Vec<Batch>,
    generation: u64,
    edges: bool,
}

const SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    key_dir: vec4<f32>,
    fill_dir: vec4<f32>,
    accent: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let cc = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = cc * 12.92;
    let hi = 1.055 * pow(cc, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, cc <= vec3<f32>(0.0031308));
}
fn out_color(c: vec3<f32>, a: f32) -> vec4<f32> {
    // egui blends premultiplied alpha.
    if (g.params.x > 0.5) { return vec4<f32>(c * a, a); }
    return vec4<f32>(to_srgb(c) * a, a);
}

struct Inst {
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) color: vec4<f32>,
    @location(8) flags: u32,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) @interpolate(flat) flags: u32,
};

@vertex
fn vs_mesh(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) vcolor: vec4<f32>, inst: Inst) -> MeshOut {
    let m = mat4x4<f32>(inst.m0, inst.m1, inst.m2, inst.m3);
    let w = m * vec4<f32>(pos, 1.0);
    var o: MeshOut;
    o.clip = g.view_proj * w;
    o.world = w.xyz;
    o.normal = (m * vec4<f32>(normal, 0.0)).xyz;
    o.color = to_linear(select(inst.color.rgb, vcolor.rgb, vcolor.a > 0.5));
    o.flags = inst.flags;
    return o;
}

@fragment
fn fs_mesh(i: MeshOut) -> @location(0) vec4<f32> {
    var n = normalize(i.normal);
    let v = normalize(g.eye.xyz - i.world);
    if (dot(n, v) < 0.0) { n = -n; }
    var base = i.color;
    if ((i.flags & 1u) != 0u) {
        let grey = dot(base, vec3<f32>(0.2126, 0.7152, 0.0722));
        base = mix(base, vec3<f32>(grey), 0.3) * 0.62;
    }
    if ((i.flags & 4u) != 0u) {
        base = mix(base, g.accent.rgb, 0.30 + 0.15 * g.params.z);
    }
    let key = max(dot(n, g.key_dir.xyz), 0.0);
    let fill = max(dot(n, g.fill_dir.xyz), 0.0);
    let sky = 0.5 + 0.5 * n.y;
    let ambient = 0.20 + 0.14 * sky;
    let h = normalize(g.key_dir.xyz + v);
    let spec = pow(max(dot(n, h), 0.0), 48.0) * 0.18;
    var c = base * (ambient + 0.78 * key + 0.22 * fill) + vec3<f32>(spec);
    let rim = pow(1.0 - max(dot(n, v), 0.0), 2.5);
    if ((i.flags & 2u) != 0u) {
        c = c + g.accent.rgb * (0.10 + 0.55 * rim);
    }
    if ((i.flags & 4u) != 0u) {
        c = c + g.accent.rgb * 0.6 * rim;
    }
    return out_color(c, 1.0);
}

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_edge(@location(0) pos: vec3<f32>, inst: Inst) -> LineOut {
    let m = mat4x4<f32>(inst.m0, inst.m1, inst.m2, inst.m3);
    var w = (m * vec4<f32>(pos, 1.0)).xyz;
    // Pull the line toward the camera so it wins against its own faces.
    let to_eye = g.eye.xyz - w;
    w = w + normalize(to_eye) * min(0.03, length(to_eye) * 0.002);
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(w, 1.0);
    let c = inst.color.rgb;
    let lum = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    var e = select(c * 0.4, c * 0.5 + vec3<f32>(0.39), lum < 0.235);
    var a = g.params.y;
    if ((inst.flags & 1u) != 0u) { a = a * 0.5; }
    o.color = vec4<f32>(to_linear(e), a);
    return o;
}

@vertex
fn vs_line(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var o: LineOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.color = vec4<f32>(to_linear(color.rgb), color.a);
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    return out_color(i.color.rgb, i.color.a);
}
"#;

fn instance_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Unorm8x4, 8 => Uint32
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Instance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRS,
    }
}

fn blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    label: &str,
    vs: &str,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    topology: wgpu::PrimitiveTopology,
    format: wgpu::TextureFormat,
    depth_write: bool,
    depth_compare: wgpu::CompareFunction,
    fs: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: MSAA,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(blend()),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("studforge_viewer"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer_globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer_globals_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer_globals_group"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        const MESH_ATTRS: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4];
        let mesh_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRS,
        };
        const EDGE_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
        let edge_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<EdgeVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &EDGE_ATTRS,
        };
        const LINE_ATTRS: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &LINE_ATTRS,
        };
        use wgpu::CompareFunction as C;
        use wgpu::PrimitiveTopology as T;
        let mesh_pipeline = pipeline(
            device,
            &layout,
            &module,
            "viewer_mesh",
            "vs_mesh",
            &[Some(mesh_layout), Some(instance_layout())],
            T::TriangleList,
            format,
            true,
            C::Less,
            "fs_mesh",
        );
        let edge_pipeline = pipeline(
            device,
            &layout,
            &module,
            "viewer_edges",
            "vs_edge",
            &[Some(edge_layout), Some(instance_layout())],
            T::LineList,
            format,
            false,
            C::LessEqual,
            "fs_line",
        );
        let line_pipeline = pipeline(
            device,
            &layout,
            &module,
            "viewer_lines",
            "vs_line",
            &[Some(line_layout.clone())],
            T::LineList,
            format,
            false,
            C::LessEqual,
            "fs_line",
        );
        let overlay_pipeline = pipeline(
            device,
            &layout,
            &module,
            "viewer_overlay",
            "vs_line",
            &[Some(line_layout)],
            T::LineList,
            format,
            false,
            C::Always,
            "fs_line",
        );
        Self {
            mesh_pipeline,
            edge_pipeline,
            line_pipeline,
            overlay_pipeline,
            globals_buf,
            bind_group,
            meshes: HashMap::new(),
            instance_buf: None,
            line_buf: None,
            line_count: 0,
            overlay_buf: None,
            overlay_count: 0,
            batches: Vec::new(),
            generation: u64::MAX,
            edges: true,
        }
    }

    fn upload_mesh(device: &wgpu::Device, mesh: &PartMesh, palette: &Palette) -> Option<GpuMesh> {
        if mesh.indices.is_empty() || mesh.positions.is_empty() {
            return None;
        }
        let mut cache: HashMap<u16, [u8; 4]> = HashMap::new();
        let verts: Vec<MeshVertex> = mesh
            .positions
            .iter()
            .enumerate()
            .map(|(i, &pos)| {
                let normal = mesh.normals.get(i).copied().unwrap_or([0.0, -1.0, 0.0]);
                let code = mesh.colors.get(i).copied().unwrap_or(MAIN_COLOR);
                let color = if code == MAIN_COLOR {
                    [0; 4]
                } else {
                    // Codes the palette doesn't know (printed details, edge
                    // colours) fall back to the part's colour, like sf-render.
                    *cache
                        .entry(code)
                        .or_insert_with(|| match palette.get(code) {
                            Some(c) => [c.rgb[0], c.rgb[1], c.rgb[2], 255],
                            None => [0; 4],
                        })
                };
                MeshVertex { pos, normal, color }
            })
            .collect();
        let n = verts.len() as u32;
        let indices: Vec<u32> = mesh.indices.iter().copied().filter(|&i| i < n).collect();
        let indices: Vec<u32> = indices
            .as_chunks::<3>()
            .0
            .iter()
            .flatten()
            .copied()
            .collect();
        if indices.is_empty() {
            return None;
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("part_vertices"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("part_indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let edges: Vec<EdgeVertex> = mesh
            .edges
            .iter()
            .flat_map(|e| [EdgeVertex { pos: e[0] }, EdgeVertex { pos: e[1] }])
            .collect();
        let ebuf = (!edges.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("part_edges"),
                contents: bytemuck::cast_slice(&edges),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        Some(GpuMesh {
            vbuf,
            ibuf,
            index_count: indices.len() as u32,
            ebuf,
            edge_vertex_count: edges.len() as u32,
        })
    }

    fn upload_scene(&mut self, device: &wgpu::Device, scene: &Scene) {
        self.batches.clear();
        let mut all: Vec<Instance> = Vec::new();
        for (part, mesh, inst) in &scene.batches {
            if inst.is_empty() {
                continue;
            }
            if !self.meshes.contains_key(part) {
                match Self::upload_mesh(device, mesh, &scene.palette) {
                    Some(m) => {
                        self.meshes.insert(part.clone(), m);
                    }
                    None => continue,
                }
            }
            let start = all.len() as u32;
            all.extend_from_slice(inst);
            self.batches.push(Batch {
                part: part.clone(),
                range: start..all.len() as u32,
            });
        }
        self.instance_buf = (!all.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewer_instances"),
                contents: bytemuck::cast_slice(&all),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let mk = |v: &[LineVertex], label: &str| {
            (!v.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(v),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        self.line_buf = mk(&scene.lines, "viewer_lines");
        self.line_count = scene.lines.len() as u32;
        self.overlay_buf = mk(&scene.overlay_lines, "viewer_overlay");
        self.overlay_count = scene.overlay_lines.len() as u32;
        self.edges = scene.edges;
        self.generation = scene.generation;
    }
}

/// Install the renderer into egui's callback resources.
pub fn install(rs: &egui_wgpu::RenderState) {
    let r = Renderer::new(&rs.device, rs.target_format);
    rs.renderer.write().callback_resources.insert(r);
}

impl CallbackTrait for ViewerCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<Renderer>() else {
            return Vec::new();
        };
        if r.generation != self.scene.generation {
            r.upload_scene(device, &self.scene);
        }
        queue.write_buffer(&r.globals_buf, 0, bytemuck::bytes_of(&self.globals));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(r) = resources.get::<Renderer>() else {
            return;
        };
        pass.set_bind_group(0, &r.bind_group, &[]);
        // Ground grid first so parts cover it; it doesn't write depth.
        if let Some(buf) = &r.line_buf {
            pass.set_pipeline(&r.line_pipeline);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..r.line_count, 0..1);
        }
        let Some(inst) = &r.instance_buf else { return };
        pass.set_pipeline(&r.mesh_pipeline);
        pass.set_vertex_buffer(1, inst.slice(..));
        for b in &r.batches {
            let Some(m) = r.meshes.get(&b.part) else {
                continue;
            };
            pass.set_vertex_buffer(0, m.vbuf.slice(..));
            pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..m.index_count, 0, b.range.clone());
        }
        if r.edges {
            pass.set_pipeline(&r.edge_pipeline);
            pass.set_vertex_buffer(1, inst.slice(..));
            for b in &r.batches {
                let Some(m) = r.meshes.get(&b.part) else {
                    continue;
                };
                let Some(e) = &m.ebuf else { continue };
                pass.set_vertex_buffer(0, e.slice(..));
                pass.draw(0..m.edge_vertex_count, b.range.clone());
            }
        }
        if let Some(buf) = &r.overlay_buf {
            pass.set_pipeline(&r.overlay_pipeline);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..r.overlay_count, 0..1);
        }
    }
}
