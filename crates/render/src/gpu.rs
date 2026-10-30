//! wgpu plumbing: one fullscreen pass that reads the emulated framebuffer and
//! paints it through the CRT effect chain.

use std::sync::Arc;

use cread_core::Viewport;
use winit::window::Window;

use crate::raster::Framebuffer;

const SHADER: &str = include_str!("crt.wgsl");
const BLOOM_SHADER: &str = include_str!("bloom.wgsl");

/// Knobs for the CRT effect chain. All values are 0..1 unless noted.
#[derive(Clone, Copy, Debug)]
pub struct CrtSettings {
    /// How dark the empty half of each scanline pair goes.
    pub scanline: f32,
    /// Local phosphor bloom strength.
    pub bloom: f32,
    /// Overall brightness.
    pub brightness: f32,
}

impl Default for CrtSettings {
    fn default() -> Self {
        Self {
            scanline: 0.45,
            bloom: 0.45,
            brightness: 1.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CrtUniforms {
    vres: [f32; 2],
    /// Device pixels per emulated pixel; the beam profile needs it.
    scale: f32,
    scanline: f32,
    bloom: f32,
    brightness: f32,
    /// Keep the buffer at 32 bytes; see the matching WGSL struct.
    _pad: [f32; 2],
}

const _: () = assert!(
    std::mem::size_of::<CrtUniforms>() == 32,
    "CrtUniforms must stay in sync with the WGSL struct"
);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomUniforms {
    texel: [f32; 2],
    dir: [f32; 2],
    _pad: [f32; 4],
}

const _: () = assert!(
    std::mem::size_of::<BloomUniforms>() == 32,
    "BloomUniforms must stay in sync with the WGSL struct"
);

/// What happened when we tried to get a frame.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameStatus {
    Presented,
    /// Compositor says we must reconfigure; caller should call `resize`.
    Reconfigure,
    /// Skip this frame.
    Skipped,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    bloom_pipeline: wgpu::RenderPipeline,
    bloom_layout: wgpu::BindGroupLayout,
    bloom_tex: [wgpu::Texture; 2],
    bloom_view: [wgpu::TextureView; 2],
    bloom_size: (u32, u32),
    bloom_uniforms: [wgpu::Buffer; 2],
    /// [0] text -> bloom[0] (horizontal), [1] bloom[0] -> bloom[1] (vertical)
    bloom_bind: [wgpu::BindGroup; 2],
    tex_size: (u32, u32),
    adapter_info: String,
}

impl Renderer {
    pub fn new(window: Arc<Window>, backends: wgpu::Backends) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot create surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| format!("no suitable adapter: {e}"))?;

        let info = adapter.get_info();
        let adapter_info = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("cread"),
            ..Default::default()
        }))
        .map_err(|e| format!("cannot create device: {e}"))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface has no usable configuration")?;
        // Prefer a non-sRGB target so palette values are written verbatim; an
        // sRGB surface would re-encode our already-sRGB colours.
        if let Some(linear) = surface
            .get_capabilities(&adapter)
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
        {
            config.format = linear;
        }
        config.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &config);

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crt"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crt"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crt"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crt uniforms"),
            size: std::mem::size_of::<CrtUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (texture, view) = create_target(&device, 640, 400);

        // Bloom: two blur passes at half resolution, then composited on top.
        let bloom_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_SHADER.into()),
        });
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bloom"),
                bind_group_layouts: &[Some(&bloom_layout)],
                immediate_size: 0,
            });
        let bloom_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bloom"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bloom_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &bloom_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let bloom_size = bloom_size_for(640, 400);
        let bloom_tex = [
            create_bloom_target(&device, bloom_size.0, bloom_size.1),
            create_bloom_target(&device, bloom_size.0, bloom_size.1),
        ];
        let bloom_view = [
            bloom_tex[0].create_view(&wgpu::TextureViewDescriptor::default()),
            bloom_tex[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];
        let bloom_uniforms = [
            make_uniform_buffer(&device, "bloom h"),
            make_uniform_buffer(&device, "bloom v"),
        ];
        let bloom_bind = [
            make_bloom_bind(&device, &bloom_layout, &view, &linear, &bloom_uniforms[0]),
            make_bloom_bind(
                &device,
                &bloom_layout,
                &bloom_view[0],
                &linear,
                &bloom_uniforms[1],
            ),
        ];

        let bind_group = make_bind_group(
            &device,
            &layout,
            &view,
            &bloom_view[1],
            &nearest,
            &linear,
            &uniforms,
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            layout,
            nearest,
            linear,
            uniforms,
            bloom_pipeline,
            bloom_layout,
            bloom_tex,
            bloom_view,
            bloom_size,
            bloom_uniforms,
            bloom_bind,
            texture,
            view,
            bind_group,
            tex_size: (640, 400),
            adapter_info,
        })
    }

    pub fn adapter_info(&self) -> &str {
        &self.adapter_info
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Reconfigure the swap chain for a new physical surface size.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Upload a new emulated framebuffer, recreating the texture when the raster
    /// size changed (window resize, scale change).
    pub fn upload(&mut self, fb: &Framebuffer) {
        let size = (fb.width(), fb.height());
        if size != self.tex_size {
            let (texture, view) = create_target(&self.device, size.0, size.1);
            self.view = view;
            self.texture = texture;
            self.tex_size = size;

            let bloom_size = bloom_size_for(size.0, size.1);
            self.bloom_size = bloom_size;
            self.bloom_tex = [
                create_bloom_target(&self.device, bloom_size.0, bloom_size.1),
                create_bloom_target(&self.device, bloom_size.0, bloom_size.1),
            ];
            self.bloom_view = [
                self.bloom_tex[0].create_view(&wgpu::TextureViewDescriptor::default()),
                self.bloom_tex[1].create_view(&wgpu::TextureViewDescriptor::default()),
            ];
            self.bloom_bind = [
                make_bloom_bind(
                    &self.device,
                    &self.bloom_layout,
                    &self.view,
                    &self.linear,
                    &self.bloom_uniforms[0],
                ),
                make_bloom_bind(
                    &self.device,
                    &self.bloom_layout,
                    &self.bloom_view[0],
                    &self.linear,
                    &self.bloom_uniforms[1],
                ),
            ];
            self.bind_group = make_bind_group(
                &self.device,
                &self.layout,
                &self.view,
                &self.bloom_view[1],
                &self.nearest,
                &self.linear,
                &self.uniforms,
            );
        }

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            fb.data(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(fb.width() * 4),
                rows_per_image: Some(fb.height()),
            },
            wgpu::Extent3d {
                width: fb.width(),
                height: fb.height(),
                depth_or_array_layers: 1,
            },
        );
    }

    fn write_uniforms(&mut self, scale: u32, settings: &CrtSettings) {
        let uniforms = CrtUniforms {
            vres: [self.tex_size.0 as f32, self.tex_size.1 as f32],
            scale: scale as f32,
            scanline: settings.scanline,
            bloom: settings.bloom,
            brightness: settings.brightness,
            _pad: [0.0; 2],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let texel = [
            1.0 / self.bloom_size.0.max(1) as f32,
            1.0 / self.bloom_size.1.max(1) as f32,
        ];
        for (i, dir) in [[1.0, 0.0], [0.0, 1.0]].into_iter().enumerate() {
            let bloom = BloomUniforms {
                texel,
                dir,
                _pad: [0.0; 4],
            };
            self.queue
                .write_buffer(&self.bloom_uniforms[i], 0, bytemuck::bytes_of(&bloom));
        }
    }

    /// Blur the frame into the two bloom targets: horizontal, then vertical.
    fn encode_bloom(&self, encoder: &mut wgpu::CommandEncoder) {
        for i in 0..2 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_view[i],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.bloom_pipeline);
            pass.set_bind_group(0, &self.bloom_bind[i], &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Record the CRT pass into `target` at the viewport rect.
    fn encode_crt_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        vp: &Viewport,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crt"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // The raster occupies exactly `virt * scale` physical pixels; the
        // leftover border stays black. Setting a viewport (instead of doing the
        // mapping in the shader) keeps the emulated pixels aligned to the real
        // pixel grid.
        let (w, h) = vp.scaled_size();
        pass.set_viewport(
            vp.offset_x as f32,
            vp.offset_y as f32,
            w as f32,
            h as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draw and present one frame.
    pub fn render(&mut self, vp: &Viewport, settings: &CrtSettings) -> Result<FrameStatus, String> {
        self.write_uniforms(vp.scale, settings);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(FrameStatus::Reconfigure);
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return Ok(FrameStatus::Skipped),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("crt") });

        self.encode_bloom(&mut encoder);
        self.encode_crt_pass(&mut encoder, &view, vp);

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(FrameStatus::Presented)
    }

    /// Render one frame into a CPU-readable image, for `--screenshot` and the
    /// visual regression tests. Returns tightly packed RGBA8.
    pub fn render_offscreen(
        &mut self,
        vp: &Viewport,
        settings: &CrtSettings,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        self.write_uniforms(vp.scale, settings);

        let format = self.config.format;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("screenshot"),
            });
        self.encode_bloom(&mut encoder);
        self.encode_crt_pass(&mut encoder, &view, vp);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| format!("device poll failed: {e}"))?;
        rx.recv()
            .map_err(|_| "readback channel closed".to_string())?
            .map_err(|e| format!("buffer map failed: {e}"))?;

        let data = slice
            .get_mapped_range()
            .map_err(|e| format!("cannot map readback buffer: {e}"))?;
        let mut out = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            let line = &data[start..start + unpadded as usize];
            if format == wgpu::TextureFormat::Bgra8Unorm
                || format == wgpu::TextureFormat::Bgra8UnormSrgb
            {
                for px in line.as_chunks::<4>().0 {
                    out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                }
            } else {
                out.extend_from_slice(line);
            }
        }
        drop(data);
        readback.unmap();

        Ok((out, width, height))
    }
}

fn create_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("emulated framebuffer"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    bloom: &wgpu::TextureView,
    nearest: &wgpu::Sampler,
    linear: &wgpu::Sampler,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("crt"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(nearest),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(linear),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(bloom),
            },
        ],
    })
}

fn make_bloom_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    linear: &wgpu::Sampler,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bloom"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(linear),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniforms.as_entire_binding(),
            },
        ],
    })
}

fn make_uniform_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: std::mem::size_of::<BloomUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The glow is computed at half the emulated resolution: wide enough to read as
/// light rather than as a fringe around every glyph.
fn bloom_size_for(width: u32, height: u32) -> (u32, u32) {
    ((width / 2).max(1), (height / 2).max(1))
}

fn create_bloom_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bloom"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}
