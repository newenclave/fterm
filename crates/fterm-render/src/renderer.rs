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
use crate::frame::{FrameInput, Instance, PaneStyle, Rect, build_frame};
use crate::overlay::build_message_box;
use crate::picture::{BrailleCell, PictureMode, braille, place, scissor, visible_marks};
use crate::tabbar::{TabBarInput, build_tab_bar};
use crate::theme::UiColors;

const ATLAS_START_SIZE: u32 = 1024;
/// Color emoji are rare, so this atlas starts small.
const COLOR_ATLAS_START_SIZE: u32 = 512;
/// A picture texture (or a Braille form) that was not drawn for this long is freed.
const PICTURE_KEEP: std::time::Duration = std::time::Duration::from_secs(30);

/// A picture to draw in a pane: from the app's store, by the id of its mark.
pub struct PanePicture<'a> {
    /// The key of its texture (unique in the window, for example the pane and the id).
    pub key: u64,
    pub picture: &'a fterm_term::picture::Picture,
    pub columns: usize,
    pub rows: usize,
}

/// A picture of this frame: its texture and where it goes.
struct PictureDraw {
    key: u64,
    picture: fterm_term::picture::Picture,
    rect: Rect,
    clip: Rect,
}

/// The texture of one picture.
struct PictureTexture {
    bind_group: wgpu::BindGroup,
    used: std::time::Instant,
}

/// The Braille form of a picture on its cells, made once.
struct BrailleForm {
    cells: Vec<BrailleCell>,
    used: std::time::Instant,
}

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
    ui: UiColors,
    padding: f32,
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
    /// Pictures: their own pipeline (one texture each), a linear sampler, the textures, and the Braille forms.
    picture_pipeline: wgpu::RenderPipeline,
    picture_layout: wgpu::BindGroupLayout,
    picture_sampler: wgpu::Sampler,
    picture_textures: std::collections::HashMap<u64, PictureTexture>,
    braille_forms: std::collections::HashMap<(u64, usize, usize), BrailleForm>,
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
        let (picture_pipeline, picture_layout) = create_picture_pipeline(device, format);
        // Pictures are scaled to their cells, so they need a linear filter.
        let picture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("picture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            fonts,
            palette: Palette::default(),
            ui: UiColors::default(),
            padding,
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
            picture_pipeline,
            picture_layout,
            picture_sampler,
            picture_textures: std::collections::HashMap::new(),
            braille_forms: std::collections::HashMap::new(),
        })
    }

    pub fn cell(&self) -> CellMetrics {
        self.fonts.cell()
    }

    /// New colors (from the config). They show on the next frame.
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// New UI colors (from the theme). They show on the next frame.
    pub fn set_ui(&mut self, ui: UiColors) {
        self.ui = ui;
    }

    /// The font size in physical pixels.
    pub fn font_size(&self) -> f32 {
        self.fonts.size_px()
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
        let (quads, split, pictures) = self.build(device, queue, &mut build);
        let pictures = self.picture_textures(device, queue, pictures);
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
            // The panes, then their pictures, then all that is above the panes (hints, dock, toasts, boxes).
            let grid = |pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>| {
                if !range.is_empty() {
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.instances.slice(..));
                    pass.draw(0..4, range);
                }
            };
            let split = split as u32;
            grid(&mut pass, 0..split);
            if let Some(buffer) = &pictures.1 {
                pass.set_pipeline(&self.picture_pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                for (i, (key, clip)) in pictures.0.iter().enumerate() {
                    let (Some(texture), Some([x, y, w, h])) =
                        (self.picture_textures.get(key), scissor(*clip, size))
                    else {
                        continue;
                    };
                    pass.set_scissor_rect(x, y, w, h);
                    pass.set_bind_group(0, &texture.bind_group, &[]);
                    pass.draw(0..4, i as u32..i as u32 + 1);
                }
                pass.set_scissor_rect(0, 0, size.0.max(1), size.1.max(1));
            }
            grid(&mut pass, split..quads.len() as u32);
        }
        queue.submit([encoder.finish()]);
    }

    /// Makes the textures of the pictures of this frame (a new picture is uploaded once), frees the ones not drawn
    /// for a while, and gives the pictures to draw (their keys and clips) with their quads.
    #[allow(clippy::type_complexity)]
    fn picture_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pictures: Vec<PictureDraw>,
    ) -> (Vec<(u64, Rect)>, Option<wgpu::Buffer>) {
        let now = std::time::Instant::now();
        self.picture_textures
            .retain(|_, t| now.duration_since(t.used) < PICTURE_KEEP);
        self.braille_forms
            .retain(|_, b| now.duration_since(b.used) < PICTURE_KEEP);
        if pictures.is_empty() {
            return (Vec::new(), None);
        }
        let mut draws = Vec::new();
        let mut instances = Vec::new();
        for draw in pictures {
            let texture = self.picture_textures.entry(draw.key).or_insert_with(|| {
                create_picture_texture(
                    device,
                    queue,
                    &self.picture_layout,
                    &self.uniforms,
                    &self.picture_sampler,
                    &draw.picture,
                )
            });
            texture.used = now;
            draws.push((draw.key, draw.clip));
            instances.push(Instance {
                rect: [draw.rect.x, draw.rect.y, draw.rect.width, draw.rect.height],
                uv: [0.0, 0.0, 1.0, 1.0],
                color: [1.0; 4],
                kind: 0,
                _pad: [0; 3],
            });
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pictures"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
        (draws, Some(buffer))
    }

    /// Builds the quads. When an atlas is full, it grows and we try again.
    fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        build: &mut impl FnMut(&mut FrameParts) -> Result<(), AtlasFull>,
    ) -> (Vec<Instance>, usize, Vec<PictureDraw>) {
        let max_texture = self.max_atlas_size;
        for _ in 0..4 {
            let (cell, padding) = (self.fonts.cell(), self.padding);
            let (mask, color, fonts) = (&mut self.mask, &mut self.color, &mut self.fonts);
            let (gamma, palette) = (&self.gamma, &self.palette);
            let ui = &self.ui;
            // True when the color atlas was the full one.
            let mut full_color = false;
            let mut glyph = |key: &GlyphKey| {
                if let Some(glyph) = mask.atlas.cached(key).or_else(|| color.atlas.cached(key)) {
                    return Ok(glyph);
                }
                match draw_glyph(fonts, key, gamma) {
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
                ui,
                pictures: Vec::new(),
                overlays_at: None,
                braille_forms: &mut self.braille_forms,
                max_texture,
            };
            let result = build(&mut parts);
            let split = parts.overlays_at.unwrap_or(parts.quads.len());
            let (quads, pictures) = (parts.quads, parts.pictures);
            match result {
                Ok(()) => return (quads, split, pictures),
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
        (Vec::new(), 0, Vec::new())
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
    ui: &'a UiColors,
    /// The pictures of the panes (drawn after the quads before `overlays_at`).
    pictures: Vec<PictureDraw>,
    overlays_at: Option<usize>,
    braille_forms: &'a mut std::collections::HashMap<(u64, usize, usize), BrailleForm>,
    max_texture: u32,
}

impl FrameParts<'_> {
    pub fn cell(&self) -> CellMetrics {
        self.cell
    }

    /// A terminal pane in `area` (window pixels), drawn as `style` says.
    pub fn pane<T: EventListener>(
        &mut self,
        term: &Term<T>,
        area: Rect,
        style: PaneStyle,
    ) -> Result<(), AtlasFull> {
        let input = FrameInput {
            cell: self.cell,
            padding: self.padding,
            palette: self.palette,
            ui: self.ui,
            focused: style.focused,
            area,
            harmonize: style.harmonize,
            program_palette: style.program_palette,
            braille: style.braille,
            underline: style.underline,
        };
        let quads = build_frame(term, &input, &mut self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// The pictures in the output of a pane drawn in `area` (after `pane`). `lookup` gives a picture by the id of
    /// its mark; `max_rows` is the height of its highest picture (so a picture whose top is scrolled away is found).
    pub fn pictures<'p, T: EventListener>(
        &mut self,
        term: &Term<T>,
        area: Rect,
        mode: PictureMode,
        max_rows: usize,
        lookup: impl Fn(u32) -> Option<PanePicture<'p>>,
    ) -> Result<(), AtlasFull> {
        if mode == PictureMode::Off {
            return Ok(());
        }
        use fterm_term::alacritty_terminal::grid::Dimensions;
        let screen = (term.columns(), term.screen_lines());
        for (id, row, column) in visible_marks(term, max_rows) {
            let Some(p) = lookup(id) else {
                continue;
            };
            let Some((rect, clip)) = place(
                row,
                column,
                (p.columns, p.rows),
                self.cell,
                area,
                self.padding,
                screen,
            ) else {
                continue;
            };
            let too_big = p.picture.width.max(p.picture.height) > self.max_texture;
            if mode == PictureMode::Pixels && !too_big {
                self.pictures.push(PictureDraw {
                    key: p.key,
                    picture: p.picture.clone(),
                    rect,
                    clip,
                });
            } else {
                self.braille(&p, rect, clip)?;
            }
        }
        Ok(())
    }

    /// The Braille form of a picture: Braille glyphs on its cells, only the cells inside `clip`.
    fn braille(&mut self, p: &PanePicture<'_>, rect: Rect, clip: Rect) -> Result<(), AtlasFull> {
        let form = self
            .braille_forms
            .entry((p.key, p.columns, p.rows))
            .or_insert_with(|| BrailleForm {
                cells: braille(
                    &p.picture.rgba,
                    (p.picture.width, p.picture.height),
                    (p.columns, p.rows),
                ),
                used: std::time::Instant::now(),
            });
        form.used = std::time::Instant::now();
        let cells = form.cells.clone();
        let cell = self.cell;
        for c in cells {
            let x = rect.x + c.column as f32 * cell.width;
            let y = rect.y + c.row as f32 * cell.height;
            if x < clip.x
                || y < clip.y
                || x + cell.width > clip.x + clip.width + 0.5
                || y + cell.height > clip.y + clip.height + 0.5
            {
                continue;
            }
            let key = GlyphKey {
                c: c.c,
                extra: None,
                bold: false,
                italic: false,
                wide: false,
                dots: true,
            };
            if let Some(g) = (self.glyph)(&key)? {
                self.quads.push(Instance {
                    rect: [
                        x + g.left as f32,
                        y + cell.baseline - g.top as f32,
                        g.width as f32,
                        g.height as f32,
                    ],
                    uv: [g.x as f32, g.y as f32, g.width as f32, g.height as f32],
                    color: linear(fterm_term::alacritty_terminal::vte::ansi::Rgb {
                        r: c.color[0],
                        g: c.color[1],
                        b: c.color[2],
                    }),
                    kind: crate::frame::KIND_GLYPH,
                    _pad: [0; 3],
                });
            }
        }
        Ok(())
    }

    /// Everything added after this is drawn above the pictures of the panes (call it after the panes).
    pub fn overlays(&mut self) {
        self.overlays_at = Some(self.quads.len());
    }

    pub fn tab_bar(&mut self, input: &TabBarInput) -> Result<(), AtlasFull> {
        let quads = build_tab_bar(input, self.ui, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// Lines between panes, and the frame of the active pane (when there are many panes).
    pub fn pane_chrome(&mut self, dividers: &[Rect], active: Option<Rect>) {
        self.quads
            .extend(crate::panes::build_pane_chrome(dividers, active, self.ui));
    }

    /// The command palette at the top of `view`.
    pub fn palette(
        &mut self,
        palette: &crate::overlay::PaletteView,
        view: Rect,
    ) -> Result<(), AtlasFull> {
        let quads =
            crate::overlay::build_palette(palette, view, self.ui, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// A grey hint after the cursor of the pane in `area`: at screen cell (`column`, `line`),
    /// at most to the end of that line.
    pub fn ghost(
        &mut self,
        text: &str,
        area: Rect,
        column: usize,
        line: usize,
        columns: usize,
    ) -> Result<(), AtlasFull> {
        let x = area.x + self.padding + column as f32 * self.cell.width;
        let y = area.y + self.padding + line as f32 * self.cell.height;
        let max = columns.saturating_sub(column);
        let quads =
            crate::overlay::build_ghost(text, x, y, max, self.ui, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// Labels over the cells of a pane in `area`: `(text, column, row)` (the hints for links).
    pub fn labels(
        &mut self,
        labels: &[(String, usize, usize)],
        area: Rect,
    ) -> Result<(), AtlasFull> {
        let (x, y) = (area.x + self.padding, area.y + self.padding);
        let quads =
            crate::overlay::build_labels(labels, x, y, self.ui, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// The dock with its panel (from `dock::layout_dock`).
    pub fn dock(
        &mut self,
        view: &crate::dock::DockView,
        layout: &crate::dock::DockLayout,
    ) -> Result<(), AtlasFull> {
        let quads = crate::dock::build_dock(view, layout, self.ui, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// Toasts at their places (from `toasts::layout_toasts`).
    pub fn toasts(
        &mut self,
        toasts: &[crate::toasts::ToastView],
        rects: &[Rect],
    ) -> Result<(), AtlasFull> {
        let quads =
            crate::toasts::build_toasts(toasts, rects, self.ui, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }

    /// A message box in the middle of `view`, on top of everything.
    pub fn message_box(&mut self, lines: &[String], view: Rect) -> Result<(), AtlasFull> {
        let quads = build_message_box(lines, view, self.ui, self.cell, &mut *self.glyph)?;
        self.quads.extend(quads);
        Ok(())
    }
}

/// Draws one glyph: builtin chars in code, all other chars with the font.
fn draw_glyph(fonts: &mut Fonts, key: &GlyphKey, gamma: &[u8; 256]) -> Option<GlyphImage> {
    if key.extra.is_none() && is_builtin(key.c) {
        let braille = if key.dots {
            BrailleStyle::Dots
        } else {
            BrailleStyle::Pixels
        };
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

/// The pipeline for pictures: the same quads, one texture per picture.
fn create_picture_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("picture.wgsl"));
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("picture"),
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
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("picture"),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("picture"),
        layout: Some(&pipeline_layout),
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
    (pipeline, layout)
}

/// Uploads a picture into its own texture (sRGB, as the file has it).
fn create_picture_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    picture: &fterm_term::picture::Picture,
) -> PictureTexture {
    let size = wgpu::Extent3d {
        width: picture.width.max(1),
        height: picture.height.max(1),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("picture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if picture.rgba.len() as u64 >= u64::from(size.width) * u64::from(size.height) * 4 {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &picture.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 4),
                rows_per_image: None,
            },
            size,
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("picture"),
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
    });
    PictureTexture {
        bind_group,
        used: std::time::Instant::now(),
    }
}
