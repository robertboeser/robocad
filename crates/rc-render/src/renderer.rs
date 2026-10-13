//! GPU resources, pipelines and frame rendering.

use std::collections::HashMap;
use std::path::Path;

use bytemuck::{Pod, Zeroable};
use glam::{DMat4, Mat4};
use rc_mesh::Mesh;
use rc_mesh::edges::FeatureEdges;
use wgpu::util::DeviceExt;

use crate::{OrbitCamera, Projection, SceneStyle};

/// Identifies an uploaded mesh (usually the mesh's content hash).
pub type MeshKey = u64;

const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const COLOR_VIEW_UNORM: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const EDGE_DEPTH_OFFSET: f32 = 0.0008;

/// How an object is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DrawStyle {
    #[default]
    Solid,
    /// Translucent grey "hole" look.
    Hole,
    /// Translucent preview (e.g. children of a group during a drag).
    Ghost,
}

/// One object to draw in a frame.
#[derive(Clone, Copy, Debug)]
pub struct DrawObject {
    pub mesh: MeshKey,
    pub model: DMat4,
    /// Linear RGBA. Alpha is used for transparent styles.
    pub color: [f32; 4],
    pub style: DrawStyle,
    pub selected: bool,
    pub hovered: bool,
}

/// Everything needed to render a frame.
#[derive(Clone, Copy, Debug)]
pub struct FrameDesc<'a> {
    pub camera: &'a OrbitCamera,
    pub objects: &'a [DrawObject],
    pub style: &'a SceneStyle,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalsUniform {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    camera_fwd: [f32; 4],
    light_key: [f32; 4],
    light_fill: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    plate: [f32; 4],
    plate_color: [f32; 4],
    grid_color: [f32; 4],
    edge_params: [f32; 4],
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    edge_color: [f32; 4],
    flags: [f32; 4],
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    sharp_edges: Option<(wgpu::Buffer, u32)>,
    diag_edges: Option<(wgpu::Buffer, u32)>,
    bytes: usize,
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    output_view: wgpu::TextureView,
}

struct Pipelines {
    background: wgpu::RenderPipeline,
    mesh: wgpu::RenderPipeline,
    mesh_transparent: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
}

/// The robocad scene renderer.
pub struct Renderer {
    pipelines: Pipelines,
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    object_layout: wgpu::BindGroupLayout,
    objects: wgpu::Buffer,
    objects_bind: wgpu::BindGroup,
    object_capacity: usize,
    object_stride: u64,
    meshes: HashMap<MeshKey, GpuMesh>,
    targets: Option<Targets>,
    /// Incremented whenever the output texture is recreated.
    generation: u64,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("meshes", &self.meshes.len())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Renderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("robocad scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(false, std::mem::size_of::<GlobalsUniform>())],
        });
        let object_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("object"),
            entries: &[uniform_entry(true, std::mem::size_of::<ObjectUniform>())],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&globals_layout), Some(&object_layout)],
            immediate_size: 0,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<GlobalsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let object_stride = (std::mem::size_of::<ObjectUniform>() as u64).div_ceil(align) * align;
        let object_capacity = 64;
        let (objects, objects_bind) =
            create_object_buffer(device, &object_layout, object_stride, object_capacity);
        let pipelines = create_pipelines(device, &layout, &shader);
        Self {
            pipelines,
            globals,
            globals_bind,
            object_layout,
            objects,
            objects_bind,
            object_capacity,
            object_stride,
            meshes: HashMap::new(),
            targets: None,
            generation: 0,
        }
    }

    /// Uploads a mesh under `key` (replacing any previous mesh with that key).
    pub fn upload_mesh(
        &mut self,
        device: &wgpu::Device,
        key: MeshKey,
        mesh: &Mesh,
        edges: &FeatureEdges,
    ) {
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(&mesh.positions),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytemuck::cast_slice(&mesh.triangles),
            usage: wgpu::BufferUsages::INDEX,
        });
        let line_buffer = |lines: &[[u32; 2]]| {
            (!lines.is_empty()).then(|| {
                (
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh edges"),
                        contents: bytemuck::cast_slice(lines),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    (lines.len() * 2) as u32,
                )
            })
        };
        let mut diag = edges.boundary.clone();
        diag.extend_from_slice(&edges.non_manifold);
        let bytes = mesh.memory_bytes() + edges.len() * 8;
        self.meshes.insert(
            key,
            GpuMesh {
                vertices,
                indices,
                index_count: (mesh.triangles.len() * 3) as u32,
                sharp_edges: line_buffer(&edges.sharp),
                diag_edges: line_buffer(&diag),
                bytes,
            },
        );
    }

    pub fn has_mesh(&self, key: MeshKey) -> bool {
        self.meshes.contains_key(&key)
    }

    pub fn remove_mesh(&mut self, key: MeshKey) {
        self.meshes.remove(&key);
    }

    /// Keeps only meshes for which `keep` returns true.
    pub fn retain_meshes(&mut self, mut keep: impl FnMut(MeshKey) -> bool) {
        self.meshes.retain(|k, _| keep(*k));
    }

    /// Approximate GPU memory used by meshes, in bytes.
    pub fn mesh_bytes(&self) -> usize {
        self.meshes.values().map(|m| m.bytes).sum()
    }

    /// Changes whenever the output texture is recreated (e.g. on resize).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The rendered image as a non-sRGB view holding sRGB-encoded bytes (what egui expects).
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.output_view)
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let tex = |label, format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = tex(
            "msaa color",
            COLOR_FORMAT,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        );
        let depth = tex(
            "depth",
            DEPTH_FORMAT,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            &[],
        );
        let resolve = tex(
            "viewport output",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            &[COLOR_VIEW_UNORM],
        );
        let output_view = resolve.create_view(&wgpu::TextureViewDescriptor {
            format: Some(COLOR_VIEW_UNORM),
            ..Default::default()
        });
        self.targets = Some(Targets {
            size,
            msaa: msaa.create_view(&wgpu::TextureViewDescriptor::default()),
            depth: depth.create_view(&wgpu::TextureViewDescriptor::default()),
            resolve_view: resolve.create_view(&wgpu::TextureViewDescriptor::default()),
            resolve,
            output_view,
        });
        self.generation += 1;
    }

    /// Renders a frame of `size` pixels into the output texture and submits it.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        frame: &FrameDesc<'_>,
    ) {
        self.ensure_targets(device, size);
        let aspect = f64::from(size[0].max(1)) / f64::from(size[1].max(1));
        queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&globals_uniform(frame, aspect, size)),
        );

        // Two uniform slots per object: normal edges and diagnostic edges.
        let visible: Vec<&DrawObject> = frame
            .objects
            .iter()
            .filter(|o| self.meshes.contains_key(&o.mesh))
            .collect();
        let slots = visible.len() * 3;
        if slots > self.object_capacity {
            self.object_capacity = slots.next_power_of_two();
            let (buf, bind) = create_object_buffer(
                device,
                &self.object_layout,
                self.object_stride,
                self.object_capacity,
            );
            self.objects = buf;
            self.objects_bind = bind;
        }
        let stride = self.object_stride as usize;
        let mut data = vec![0u8; slots.max(1) * stride];
        let style = frame.style;
        for (i, o) in visible.iter().enumerate() {
            let flags = [
                f32::from(u8::from(o.selected)),
                f32::from(u8::from(o.hovered)),
                match o.style {
                    DrawStyle::Solid => 0.0,
                    DrawStyle::Hole => 1.0,
                    DrawStyle::Ghost => 2.0,
                },
                0.0,
            ];
            let edge_color = if o.selected {
                style.selection_color
            } else {
                style.edge_color
            };
            let base = ObjectUniform {
                model: o.model.as_mat4().to_cols_array_2d(),
                color: o.color,
                edge_color,
                flags,
            };
            let slot_values = [
                base,
                ObjectUniform {
                    edge_color: style.boundary_edge_color,
                    ..base
                },
                ObjectUniform {
                    edge_color: style.non_manifold_edge_color,
                    ..base
                },
            ];
            for (k, u) in slot_values.iter().enumerate() {
                let off = (i * 3 + k) * stride;
                data[off..off + std::mem::size_of::<ObjectUniform>()]
                    .copy_from_slice(bytemuck::bytes_of(u));
            }
        }
        queue.write_buffer(&self.objects, 0, &data);

        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);

            pass.set_pipeline(&self.pipelines.background);
            pass.set_bind_group(1, &self.objects_bind, &[0]);
            pass.draw(0..3, 0..1);

            let offset = |i: usize, k: usize| ((i * 3 + k) * stride) as u32;
            let opaque = |o: &DrawObject| o.style == DrawStyle::Solid;

            pass.set_pipeline(&self.pipelines.mesh);
            for (i, o) in visible.iter().enumerate().filter(|(_, o)| opaque(o)) {
                let m = &self.meshes[&o.mesh];
                pass.set_bind_group(1, &self.objects_bind, &[offset(i, 0)]);
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.index_count, 0, 0..1);
            }

            if style.show_grid {
                pass.set_pipeline(&self.pipelines.grid);
                pass.set_bind_group(1, &self.objects_bind, &[0]);
                pass.draw(0..6, 0..1);
            }

            pass.set_pipeline(&self.pipelines.edges);
            for (i, o) in visible.iter().enumerate() {
                let m = &self.meshes[&o.mesh];
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                if (style.show_edges || o.selected)
                    && let Some((buf, n)) = &m.sharp_edges
                {
                    pass.set_bind_group(1, &self.objects_bind, &[offset(i, 0)]);
                    pass.set_index_buffer(buf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                }
                if style.show_diagnostics
                    && let Some((buf, n)) = &m.diag_edges
                {
                    pass.set_bind_group(1, &self.objects_bind, &[offset(i, 1)]);
                    pass.set_index_buffer(buf.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..*n, 0, 0..1);
                }
            }

            pass.set_pipeline(&self.pipelines.mesh_transparent);
            for (i, o) in visible.iter().enumerate().filter(|(_, o)| !opaque(o)) {
                let m = &self.meshes[&o.mesh];
                pass.set_bind_group(1, &self.objects_bind, &[offset(i, 0)]);
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.index_count, 0, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }

    /// Reads back the last rendered frame as tightly packed sRGB RGBA8 pixels.
    pub fn read_pixels(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let targets = self.targets.as_ref()?;
        let [w, h] = targets.size;
        let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
        encoder.copy_texture_to_buffer(
            targets.resolve.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h as usize {
            let start = row * padded as usize;
            out.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buffer.unmap();
        Some((w, h, out))
    }

    /// Writes the last rendered frame to a PNG file.
    pub fn save_png(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        let (w, h, pixels) = self
            .read_pixels(device, queue)
            .ok_or_else(|| std::io::Error::other("nothing rendered yet"))?;
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(std::io::Error::other)?;
        writer
            .write_image_data(&pixels)
            .map_err(std::io::Error::other)?;
        Ok(())
    }
}

fn globals_uniform(frame: &FrameDesc<'_>, aspect: f64, size: [u32; 2]) -> GlobalsUniform {
    let cam = frame.camera;
    let style = frame.style;
    let v4 = |v: glam::DVec3, w: f32| {
        let v = v.as_vec3();
        [v.x, v.y, v.z, w]
    };
    let fwd = cam.forward();
    let (right, up) = (cam.right(), cam.up());
    let key = (-fwd * 0.45 + up * 0.75 - right * 0.65).normalize();
    let fill = (-fwd * 0.5 - up * 0.1 + right * 0.85).normalize();
    let ortho = cam.projection == Projection::Orthographic;
    let view_proj: Mat4 = cam.view_proj(aspect).as_mat4();
    GlobalsUniform {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: v4(cam.eye(), cam.distance as f32),
        camera_fwd: v4(fwd, f32::from(u8::from(ortho))),
        light_key: v4(key, 0.0),
        light_fill: v4(fill, 0.0),
        bg_top: style.bg_top,
        bg_bottom: style.bg_bottom,
        plate: [
            style.plate_size[0],
            style.plate_size[1],
            style.grid_minor,
            style.grid_major,
        ],
        plate_color: style.plate_color,
        grid_color: style.grid_color,
        edge_params: [EDGE_DEPTH_OFFSET, 0.0, 0.0, 0.0],
        viewport: [size[0] as f32, size[1] as f32, 0.0, 0.0],
    }
}

fn uniform_entry(dynamic: bool, size: usize) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: wgpu::BufferSize::new(size as u64),
        },
        count: None,
    }
}

fn create_object_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("objects"),
        size: stride * capacity as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("objects"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
            }),
        }],
    });
    (buffer, bind)
}

fn create_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
) -> Pipelines {
    use wgpu::CompareFunction as C;
    use wgpu::PrimitiveTopology as T;
    let vertex_layout = [Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
    })];
    let make = |label: &str,
                vs: &str,
                fs: &str,
                buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                topology: wgpu::PrimitiveTopology,
                depth_write: bool,
                depth_compare: wgpu::CompareFunction,
                blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology,
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(depth_compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
    Pipelines {
        background: make(
            "background",
            "vs_background",
            "fs_background",
            &[],
            T::TriangleList,
            false,
            C::Always,
            None,
        ),
        mesh: make(
            "mesh",
            "vs_mesh",
            "fs_mesh",
            &vertex_layout,
            T::TriangleList,
            true,
            C::Greater,
            None,
        ),
        mesh_transparent: make(
            "mesh transparent",
            "vs_mesh",
            "fs_mesh_transparent",
            &vertex_layout,
            T::TriangleList,
            false,
            C::Greater,
            alpha,
        ),
        edges: make(
            "edges",
            "vs_edge",
            "fs_edge",
            &vertex_layout,
            T::LineList,
            false,
            C::GreaterEqual,
            alpha,
        ),
        grid: make(
            "grid",
            "vs_grid",
            "fs_grid",
            &[],
            T::TriangleList,
            false,
            C::Greater,
            alpha,
        ),
    }
}

/// Creates a GPU device without a window (for PNG rendering and visual tests).
/// Returns `None` if no suitable adapter exists (e.g. on headless CI).
pub fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}
