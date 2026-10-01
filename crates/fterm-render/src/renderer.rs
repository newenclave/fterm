//! GPU part: the pipeline, the atlas texture, and drawing one frame.

use fterm_term::alacritty_terminal::event::EventListener;
use fterm_term::alacritty_terminal::term::Term;
use fterm_term::alacritty_terminal::vte::ansi::NamedColor;
use fterm_term::colors::Palette;
use wgpu::util::DeviceExt;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphAtlas, GlyphKey};
use crate::builtin::{BrailleStyle, builtin_glyph, is_builtin};
use crate::color::{linear, text_alpha};
use crate::font::{CellMetrics, Fonts, GlyphImage, ImageKind};
use crate::frame::{FrameInput, Instance, Rect, build_frame};
use crate::overlay::build_message_box;
use crate::tabbar::{TabBarInput, build_tab_bar};

const ATLAS_START_SIZE: u32 = 1024;
/// Color emoji are rare, so this atlas starts small.
const COLOR_ATLAS_START_SIZE: u32 = 512;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    mask_atlas_size: [f32; 2],
    color_atlas_size: [f32; 2],
    _pad: [f32; 2],
}

/// A glyph atlas and its texture.
struct AtlasTexture {
    atlas: GlyphAtlas,
    texture: wgpu::Texture,
    format: wgpu::TextureFormat,
}

impl AtlasTexture {
    fn new(device: &wgpu::Device, size: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            atlas: GlyphAtlas::new(size),
            texture: create_atlas_texture(device, size, format),
            format,
        }
    }

    /// Finds the glyph, or puts `image` into the atlas and the texture.
    fn get(
        &mut self,
        queue: &wgpu::Queue,
        key: &GlyphKey,
        image: Option<GlyphImage>,
    ) -> Result<Option<AtlasGlyph>, AtlasFull> {
        let texture = &self.texture;
        self.atlas.get(
            key,
            || image,
            |place, image| upload_glyph(queue, texture, place, image),
        )
    }
}

pub struct Renderer {
    fonts: Fonts,
    palette: Palette,
    padding: f32,
    braille: BrailleStyle,
    /// Font glyph alpha after `text_alpha`, for every alpha value.
    gamma: [u8; 256],
    mask: AtlasTexture,
    color: AtlasTexture,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
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
        let mask = AtlasTexture::new(device, ATLAS_START_SIZE, wgpu::TextureFormat::R8Unorm);
        let color = AtlasTexture::new(
            device,
            COLOR_ATLAS_START_SIZE,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let bind_group = create_bind_group(
            device,
            &bind_group_layout,
            &uniforms,
            &mask,
            &color,
            &sampler,
        );
        let instance_capacity = 1024;
        let instances = create_instance_buffer(device, instance_capacity);

        Ok(Self {
            fonts,
            palette: Palette::default(),
            padding,
            braille: BrailleStyle::default(),
            gamma: std::array::from_fn(|a| (text_alpha(a as f32 / 255.0) * 255.0).round() as u8),
            mask,
            color,
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
        self.mask = AtlasTexture::new(device, ATLAS_START_SIZE, self.mask.format);
        self.color = AtlasTexture::new(device, COLOR_ATLAS_START_SIZE, self.color.format);
        self.update_bind_group(device);
        Ok(())
    }

    /// Draws one frame into `view`. `size` is the view size in pixels.
    /// `build` adds the parts of the frame (tab bar, panes, message box). It can run more than
    /// once: when a glyph atlas gets full, the atlas grows and the frame is built again.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        mut build: impl FnMut(&mut FrameParts) -> Result<(), AtlasFull>,
    ) {
        let quads = self.build(device, queue, &mut build);
        let background = linear(self.palette.get(
            NamedColor::Background as usize,
            &fterm_term::alacritty_terminal::term::color::Colors::default(),
        ));

        if quads.len() > self.instance_capacity {
            self.instance_capacity = quads.len().next_power_of_two();
            self.instances = create_instance_buffer(device, self.instance_capacity);
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&quads));
        let mask_size = self.mask.atlas.size() as f32;
        let color_size = self.color.atlas.size() as f32;
        let uniforms = Uniforms {
            viewport: [size.0 as f32, size.1 as f32],
            mask_atlas_size: [mask_size, mask_size],
            color_atlas_size: [color_size, color_size],
            _pad: [0.0; 2],
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

    /// Builds the quads. When an atlas is full, it grows and we try again.
    fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        build: &mut impl FnMut(&mut FrameParts) -> Result<(), AtlasFull>,
    ) -> Vec<Instance> {
        for _ in 0..4 {
            let (cell, padding) = (self.fonts.cell(), self.padding);
            let (mask, color, fonts) = (&mut self.mask, &mut self.color, &mut self.fonts);
            let (braille, gamma, palette) = (self.braille, &self.gamma, &self.palette);
            // True when the color atlas was the full one.
            let mut full_color = false;
            let mut glyph = |key: &GlyphKey| {
                if let Some(glyph) = mask.atlas.cached(key).or_else(|| color.atlas.cached(key)) {
                    return Ok(glyph);
                }
                match draw_glyph(fonts, key, braille, gamma) {
                    Some(image) if image.kind == ImageKind::Color => {
                        let glyph = color.get(queue, key, Some(image));
                        full_color = glyph.is_err();
                        glyph
                    }
                    image => mask.get(queue, key, image),
                }
            };
            let mut parts = FrameParts {
                quads: Vec::new(),
                glyph: &mut glyph,
                cell,
                padding,
                palette,
            };
            let result = build(&mut parts);
            let quads = parts.quads;
            match result {
                Ok(()) => return quads,
                Err(AtlasFull) => {
                    let atlas = if full_color {
                        &mut self.color
                    } else {
                        &mut self.mask
                    };
                    let size = (atlas.atlas.size() * 2).min(self.max_atlas_size);
                    tracing::debug!(
                        size,
                        color = full_color,
                        "glyph atlas is full, making it bigger"
                    );
                    *atlas = AtlasTexture::new(device, size, atlas.format);
                    self.update_bind_group(device);
                }
            }
        }
        tracing::warn!("too many glyphs for the atlas, skipping this frame");
        Vec::new()
    }

    fn update_bind_group(&mut self, device: &wgpu::Device) {
        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            &self.uniforms,
            &self.mask,
            &self.color,
            &self.sampler,
        );
    }
}

/// The parts of one frame. The app adds them in drawing order (later parts are on top).
pub struct FrameParts<'a> {
    quads: Vec<Instance>,
    glyph: &'a mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
    cell: CellMetrics,
    padding: f32,
    palette: &'a Palette,
}

impl FrameParts<'_> {
    pub fn cell(&self) -> CellMetrics {
        self.cell
    }

    /// A terminal pane in `area` (window pixels).
    pub fn pane<T: EventListener>(
        &mut self,
        term: &Term<T>,
        area: Rect,
        focused: bool,
    ) -> Result<(), AtlasFull> {
        let input = FrameInput {
            cell: self.cell,
            padding: self.padding,
            palette: self.palette,
            focused,
            area,
        };
        let quads = build_frame(term, &input, &mut self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    pub fn tab_bar(&mut self, input: &TabBarInput) -> Result<(), AtlasFull> {
        let quads = build_tab_bar(input, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// A message box in the middle of `view`, on top of everything.
    pub fn message_box(&mut self, lines: &[String], view: Rect) -> Result<(), AtlasFull> {
        let quads = build_message_box(lines, view, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }
}

/// Draws one glyph: builtin chars in code, all other chars with the font.
fn draw_glyph(
    fonts: &mut Fonts,
    key: &GlyphKey,
    braille: BrailleStyle,
    gamma: &[u8; 256],
) -> Option<GlyphImage> {
    if key.extra.is_none() && is_builtin(key.c) {
        return builtin_glyph(key.c, fonts.cell(), braille);
    }
    let cells = if key.wide { 2 } else { 1 };
    let mut image = fonts.rasterize(&key.text(), key.bold, key.italic, cells)?;
    tracing::debug!(
        text = %key.text().escape_unicode(),
        cells,
        width = image.width,
        height = image.height,
        kind = ?image.kind,
        "new glyph"
    );
    if image.kind == ImageKind::Mask {
        for a in &mut image.data {
            *a = gamma[*a as usize];
        }
    }
    Some(image)
}

fn upload_glyph(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    place: &AtlasGlyph,
    image: &GlyphImage,
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
        &image.data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(match image.kind {
                ImageKind::Mask => image.width,
                ImageKind::Color => image.width * 4,
            }),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
    );
}

fn create_atlas_texture(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
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
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    mask: &AtlasTexture,
    color: &AtlasTexture,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = mask
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let color_view = color
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
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
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&color_view),
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
