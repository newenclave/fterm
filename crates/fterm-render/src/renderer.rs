//! GPU part: the pipeline, the atlas texture, and drawing one frame.

use fterm_term::alacritty_terminal::event::EventListener;
use fterm_term::alacritty_terminal::term::Term;
use fterm_term::alacritty_terminal::vte::ansi::NamedColor;
use fterm_term::colors::Palette;
use wgpu::util::DeviceExt;

use crate::atlas::{AtlasFull, GlyphAtlas};
use crate::color::linear;
use crate::font::{CellMetrics, Fonts};
use crate::frame::{FrameInput, Instance, build_frame};

const ATLAS_START_SIZE: u32 = 1024;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    atlas_size: [f32; 2],
}

pub struct Renderer {
    fonts: Fonts,
    palette: Palette,
    padding: f32,
    atlas: GlyphAtlas,
    atlas_texture: wgpu::Texture,
    max_atlas_size: u32,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_capacity: usize,
}

impl Renderer {
    /// `font_px` and `padding` are in physical pixels (already times the scale factor).
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        font_px: f32,
        padding: f32,
    ) -> anyhow::Result<Self> {
        let fonts = Fonts::new(font_px)?;
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        // Glyphs are drawn at their exact pixel size, so nearest sampling is right.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas_texture = create_atlas_texture(device, ATLAS_START_SIZE);
        let bind_group = create_bind_group(
            device,
            &bind_group_layout,
            &uniforms,
            &atlas_texture,
            &sampler,
        );
        let instance_capacity = 1024;
        let instances = create_instance_buffer(device, instance_capacity);

        Ok(Self {
            fonts,
            palette: Palette::default(),
            padding,
            atlas: GlyphAtlas::new(ATLAS_START_SIZE),
            atlas_texture,
            max_atlas_size: device.limits().max_texture_dimension_2d,
            pipeline,
            bind_group_layout,
            bind_group,
            sampler,
            uniforms,
            instances,
            instance_capacity,
        })
    }

    pub fn cell(&self) -> CellMetrics {
        self.fonts.cell()
    }

    pub fn padding(&self) -> f32 {
        self.padding
    }

    /// Changes the font size and padding (for example after a DPI change). All glyphs are drawn again.
    pub fn set_font_size(
        &mut self,
        device: &wgpu::Device,
        font_px: f32,
        padding: f32,
    ) -> anyhow::Result<()> {
        self.fonts = Fonts::new(font_px)?;
        self.padding = padding;
        self.reset_atlas(device, ATLAS_START_SIZE);
        Ok(())
    }

    /// Draws the terminal into `view`. `size` is the view size in pixels.
    pub fn render<T: EventListener>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        term: &Term<T>,
        focused: bool,
    ) {
        let quads = self.build(device, queue, term, focused);
        let background = linear(
            self.palette
                .get(NamedColor::Background as usize, term.colors()),
        );

        if quads.len() > self.instance_capacity {
            self.instance_capacity = quads.len().next_power_of_two();
            self.instances = create_instance_buffer(device, self.instance_capacity);
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&quads));
        let atlas_size = self.atlas.size() as f32;
        let uniforms = Uniforms {
            viewport: [size.0 as f32, size.1 as f32],
            atlas_size: [atlas_size, atlas_size],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(background[0]),
                            g: f64::from(background[1]),
                            b: f64::from(background[2]),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !quads.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                pass.draw(0..4, 0..quads.len() as u32);
            }
        }
        queue.submit([encoder.finish()]);
    }

    /// Builds the quads. When the atlas is full, it grows and we try again.
    fn build<T: EventListener>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        term: &Term<T>,
        focused: bool,
    ) -> Vec<Instance> {
        for _ in 0..3 {
            let input = FrameInput {
                cell: self.fonts.cell(),
                padding: self.padding,
                palette: &self.palette,
                focused,
            };
            let (atlas, fonts, texture) = (&mut self.atlas, &mut self.fonts, &self.atlas_texture);
            let result = build_frame(term, &input, &mut |key| {
                atlas.get(
                    key,
                    || fonts.rasterize(key.c, key.bold, key.italic),
                    |place, image| upload_glyph(queue, texture, place, image),
                )
            });
            match result {
                Ok(quads) => return quads,
                Err(AtlasFull) => {
                    let size = (self.atlas.size() * 2).min(self.max_atlas_size);
                    tracing::debug!(size, "glyph atlas is full, making it bigger");
                    self.reset_atlas(device, size);
                }
            }
        }
        tracing::warn!("too many glyphs for the atlas, skipping this frame");
        Vec::new()
    }

    fn reset_atlas(&mut self, device: &wgpu::Device, size: u32) {
        self.atlas = GlyphAtlas::new(size);
        self.atlas_texture = create_atlas_texture(device, size);
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            &self.uniforms,
            &self.atlas_texture,
            &self.sampler,
        );
    }
}

fn upload_glyph(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    place: &crate::atlas::AtlasGlyph,
    image: &crate::font::GlyphImage,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: place.x,
                y: place.y,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &image.alpha,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
    );
}

fn create_atlas_texture(device: &wgpu::Device, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glyph atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    atlas: &wgpu::Texture,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grid"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("instances"),
        contents: &vec![0u8; capacity * size_of::<Instance>()],
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    })
}
