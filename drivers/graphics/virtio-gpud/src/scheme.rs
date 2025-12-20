use std::fmt;
use std::sync::Arc;

use common::{dma::Dma, sgl};
use driver_graphics::kms::connector::KmsConnectorStatus;
use driver_graphics::kms::objects::{KmsObjectId, KmsObjects};
use driver_graphics::{
    Buffer as DrmBuffer, CursorPlane, GraphicsAdapter, GraphicsScheme, StandardProperties,
};
use drm_sys::{drm_mode_modeinfo, DRM_CAP_CURSOR_HEIGHT, DRM_CAP_CURSOR_WIDTH, DRM_MODE_DPMS_ON};
use graphics_ipc::v2::ipc::{DRM_CAP_DUMB_BUFFER, DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT};
use graphics_ipc::v2::Damage;

use syscall::{EINVAL, PAGE_SIZE};

use virtio_core::spec::{Buffer, ChainBuilder, DescriptorFlags};
use virtio_core::transport::{Error, Queue, Transport};

use crate::*;

impl Into<GpuRect> for Damage {
    fn into(self) -> GpuRect {
        GpuRect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug)]
pub struct VirtGpuConnector {
    display_id: u32,
}

pub struct VirtGpuFramebuffer<'a> {
    queue: Arc<Queue<'a>>,
    id: ResourceId,
    sgl: sgl::Sgl,
    width: u32,
    height: u32,
}

impl<'a> fmt::Debug for VirtGpuFramebuffer<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtGpuFramebuffer")
            .field("id", &self.id)
            .field("sgl", &self.sgl)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl DrmBuffer for VirtGpuFramebuffer<'_> {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }
}

impl Drop for VirtGpuFramebuffer<'_> {
    fn drop(&mut self) {
        futures::executor::block_on(async {
            let request = Dma::new(ResourceUnref::new(self.id)).unwrap();

            let header = Dma::new(ControlHeader::default()).unwrap();
            let command = ChainBuilder::new()
                .chain(Buffer::new(&request))
                .chain(Buffer::new(&header).flags(DescriptorFlags::WRITE_ONLY))
                .build();

            self.queue.send(command).await;
        });
    }
}

#[derive(Debug, Clone)]
pub struct Display {
    enabled: bool,
    width: u32,
    height: u32,
    edid: Vec<u8>,
    active_resource: Option<ResourceId>,
}

pub struct VirtGpuAdapter<'a> {
    pub config: &'a mut GpuConfig,
    control_queue: Arc<Queue<'a>>,
    cursor_queue: Arc<Queue<'a>>,
    transport: Arc<dyn Transport>,
    has_edid: bool,
    displays: Vec<Display>,
    hidden_cursor: Option<Arc<VirtGpuFramebuffer<'a>>>,
}

impl<'a> fmt::Debug for VirtGpuAdapter<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtGpuAdapter")
            .field("displays", &self.displays)
            .finish_non_exhaustive()
    }
}

impl VirtGpuAdapter<'_> {
    pub async fn update_displays(&mut self) -> Result<(), Error> {
        let display_info = self.get_display_info().await?;
        let raw_displays = &display_info.display_info[..self.config.num_scanouts() as usize];

        self.displays.resize(
            raw_displays.len(),
            Display {
                enabled: false,
                width: 0,
                height: 0,
                edid: vec![],
                active_resource: None,
            },
        );
        for (i, info) in raw_displays.iter().enumerate() {
            log::info!(
                "virtio-gpu: display {i} ({}x{}px)",
                info.rect.width,
                info.rect.height
            );

            self.displays[i].enabled = info.enabled != 0;

            if info.rect.width == 0 || info.rect.height == 0 {
                // QEMU gives all displays other than the first a zero width and height, but trying
                // to attach a zero sized framebuffer to the display will result an error, so
                // default to 640x480px.
                self.displays[i].width = 640;
                self.displays[i].height = 480;
            } else {
                self.displays[i].width = info.rect.width;
                self.displays[i].height = info.rect.height;
            }

            if self.has_edid {
                let edid = self.get_edid(i as u32).await?;
                self.displays[i].edid = edid.edid[..edid.size as usize].to_vec();
            }
        }

        Ok(())
    }

    async fn send_request<T>(&self, request: Dma<T>) -> Result<Dma<ControlHeader>, Error> {
        let header = Dma::new(ControlHeader::default())?;
        let command = ChainBuilder::new()
            .chain(Buffer::new(&request))
            .chain(Buffer::new(&header).flags(DescriptorFlags::WRITE_ONLY))
            .build();

        self.control_queue.send(command).await;
        Ok(header)
    }

    async fn send_request_fenced<T>(&self, request: Dma<T>) -> Result<Dma<ControlHeader>, Error> {
        let mut header = Dma::new(ControlHeader::default())?;
        header.flags |= VIRTIO_GPU_FLAG_FENCE;
        let command = ChainBuilder::new()
            .chain(Buffer::new(&request))
            .chain(Buffer::new(&header).flags(DescriptorFlags::WRITE_ONLY))
            .build();

        self.control_queue.send(command).await;
        Ok(header)
    }

    async fn get_display_info(&self) -> Result<Dma<GetDisplayInfo>, Error> {
        let header = Dma::new(ControlHeader::with_ty(CommandTy::GetDisplayInfo))?;

        let response = Dma::new(GetDisplayInfo::default())?;
        let command = ChainBuilder::new()
            .chain(Buffer::new(&header))
            .chain(Buffer::new(&response).flags(DescriptorFlags::WRITE_ONLY))
            .build();

        self.control_queue.send(command).await;
        assert!(response.header.ty == CommandTy::RespOkDisplayInfo);

        Ok(response)
    }

    async fn get_edid(&self, scanout_id: u32) -> Result<Dma<GetEdidResp>, Error> {
        let header = Dma::new(GetEdid::new(scanout_id))?;

        let response = Dma::new(GetEdidResp::new())?;
        let command = ChainBuilder::new()
            .chain(Buffer::new(&header))
            .chain(Buffer::new(&response).flags(DescriptorFlags::WRITE_ONLY))
            .build();

        self.control_queue.send(command).await;
        assert!(response.header.ty == CommandTy::RespOkEdid);

        Ok(response)
    }

    fn update_cursor(
        &mut self,
        cursor: &VirtGpuFramebuffer,
        x: i32,
        y: i32,
        hot_x: i32,
        hot_y: i32,
    ) {
        //Transfering cursor resource to host
        futures::executor::block_on(async {
            let transfer_request = Dma::new(XferToHost2d::new(
                cursor.id,
                GpuRect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                },
                0,
            ))
            .unwrap();
            let header = self.send_request_fenced(transfer_request).await.unwrap();
            assert_eq!(header.ty, CommandTy::RespOkNodata);
        });

        //Update the cursor position
        let request = Dma::new(UpdateCursor::update_cursor(x, y, hot_x, hot_y, cursor.id)).unwrap();
        futures::executor::block_on(async {
            let command = ChainBuilder::new().chain(Buffer::new(&request)).build();
            self.cursor_queue.send(command).await;
        });
    }

    fn move_cursor(&mut self, x: i32, y: i32) {
        let request = Dma::new(MoveCursor::move_cursor(x, y)).unwrap();

        futures::executor::block_on(async {
            let command = ChainBuilder::new().chain(Buffer::new(&request)).build();
            self.cursor_queue.send(command).await;
        });
    }

    fn disable_cursor(&mut self) {
        if self.hidden_cursor.is_none() {
            let (width, height) = self.hw_cursor_size().unwrap();
            let cursor = self.create_dumb_buffer(width, height);
            unsafe {
                core::ptr::write_bytes(
                    cursor.sgl.as_ptr() as *mut u8,
                    0,
                    (width * height * 4) as usize,
                );
            }
            self.hidden_cursor = Some(Arc::new(cursor));
        }
        let hidden_cursor = self.hidden_cursor.as_ref().unwrap().clone();

        self.update_cursor(&hidden_cursor, 0, 0, 0, 0);
    }
}

impl<'a> GraphicsAdapter for VirtGpuAdapter<'a> {
    type Connector = VirtGpuConnector;
    type Crtc = ();

    type Buffer = VirtGpuFramebuffer<'a>;

    fn name(&self) -> &'static [u8] {
        b"virtio-gpud"
    }

    fn desc(&self) -> &'static [u8] {
        b"VirtIO GPU"
    }

    fn init(&mut self, objects: &mut KmsObjects<Self>, standard_properties: &StandardProperties) {
        futures::executor::block_on(async {
            self.update_displays().await.unwrap();
        });

        for display_id in 0..self.config.num_scanouts.get() {
            let crtc = objects.add_crtc(());

            let connector = objects.add_connector(VirtGpuConnector { display_id }, &[crtc]);
            if self.has_edid {
                objects.add_object_property(connector, standard_properties.edid, 0);
            }
            objects.add_object_property(
                connector,
                standard_properties.dpms,
                DRM_MODE_DPMS_ON.into(),
            );
        }
    }

    fn get_cap(&self, cap: u32) -> syscall::Result<u64> {
        match cap {
            DRM_CAP_DUMB_BUFFER => Ok(1),
            DRM_CAP_CURSOR_WIDTH => Ok(64),
            DRM_CAP_CURSOR_HEIGHT => Ok(64),
            _ => Err(syscall::Error::new(EINVAL)),
        }
    }

    fn set_client_cap(&self, cap: u32, _value: u64) -> syscall::Result<()> {
        match cap {
            // FIXME hide cursor plane unless this client cap is set
            DRM_CLIENT_CAP_CURSOR_PLANE_HOTSPOT => Ok(()),
            _ => Err(syscall::Error::new(EINVAL)),
        }
    }

    fn probe_connector(
        &mut self,
        objects: &mut KmsObjects<Self>,
        standard_properties: &StandardProperties,
        id: KmsObjectId,
    ) {
        futures::executor::block_on(async {
            let mut connector = objects.get_connector(id).unwrap().lock().unwrap();
            let display = &self.displays[connector.driver_data.display_id as usize];

            connector.connection = if display.enabled {
                KmsConnectorStatus::Connected
            } else {
                KmsConnectorStatus::Disconnected
            };

            if self.has_edid {
                connector.update_from_edid(&display.edid);

                drop(connector);

                let blob = objects.add_blob(display.edid.clone());
                objects.set_object_property(id, standard_properties.edid, blob.into());
            } else {
                connector.update_from_size(display.width, display.height);
            }
        });
    }

    fn display_count(&self) -> usize {
        self.displays.len()
    }

    fn display_size(&self, display_id: usize) -> (u32, u32) {
        (
            self.displays[display_id].width,
            self.displays[display_id].height,
        )
    }

    fn create_dumb_buffer(&mut self, width: u32, height: u32) -> Self::Buffer {
        futures::executor::block_on(async {
            let bpp = 32;
            let fb_size = width as usize * height as usize * bpp / 8;
            let sgl = sgl::Sgl::new(fb_size).unwrap();

            unsafe {
                core::ptr::write_bytes(sgl.as_ptr() as *mut u8, 255, fb_size);
            }

            let res_id = ResourceId::alloc();

            // Create a host resource using `VIRTIO_GPU_CMD_RESOURCE_CREATE_2D`.
            let request = Dma::new(ResourceCreate2d::new(
                res_id,
                ResourceFormat::Bgrx,
                width,
                height,
            ))
            .unwrap();

            let header = self.send_request(request).await.unwrap();
            assert_eq!(header.ty, CommandTy::RespOkNodata);

            // Use the allocated framebuffer from the guest ram, and attach it as backing
            // storage to the resource just created, using `VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING`.

            let mut mem_entries =
                unsafe { Dma::zeroed_slice(sgl.chunks().len()).unwrap().assume_init() };
            for (entry, chunk) in mem_entries.iter_mut().zip(sgl.chunks().iter()) {
                *entry = MemEntry {
                    address: chunk.phys as u64,
                    length: chunk.length.next_multiple_of(PAGE_SIZE) as u32,
                    padding: 0,
                };
            }

            let attach_request =
                Dma::new(AttachBacking::new(res_id, mem_entries.len() as u32)).unwrap();
            let header = Dma::new(ControlHeader::default()).unwrap();
            let command = ChainBuilder::new()
                .chain(Buffer::new(&attach_request))
                .chain(Buffer::new_unsized(&mem_entries))
                .chain(Buffer::new(&header).flags(DescriptorFlags::WRITE_ONLY))
                .build();

            self.control_queue.send(command).await;
            assert_eq!(header.ty, CommandTy::RespOkNodata);

            VirtGpuFramebuffer {
                queue: self.control_queue.clone(),
                id: res_id,
                sgl,
                width,
                height,
            }
        })
    }

    fn map_dumb_buffer(&mut self, framebuffer: &Self::Buffer) -> *mut u8 {
        framebuffer.sgl.as_ptr()
    }

    fn update_plane(
        &mut self,
        objects: &KmsObjects<Self>,
        display_id: usize,
        mode: Option<drm_mode_modeinfo>,
        framebuffer: Option<&Self::Buffer>,
        damage: Damage,
    ) {
        futures::executor::block_on(async {
            objects
                .get_crtc(objects.crtc_ids()[display_id])
                .unwrap()
                .lock()
                .unwrap()
                .mode = mode;

            let Some(framebuffer) = framebuffer else {
                let scanout_request = Dma::new(SetScanout::new(
                    display_id as u32,
                    ResourceId::NONE,
                    GpuRect::new(0, 0, 0, 0),
                ))
                .unwrap();
                let header = self.send_request(scanout_request).await.unwrap();
                assert_eq!(header.ty, CommandTy::RespOkNodata);
                self.displays[display_id].active_resource = None;
                return;
            };

            let req = Dma::new(XferToHost2d::new(
                framebuffer.id,
                GpuRect {
                    x: 0,
                    y: 0,
                    width: framebuffer.width,
                    height: framebuffer.height,
                },
                0,
            ))
            .unwrap();
            let header = self.send_request(req).await.unwrap();
            assert_eq!(header.ty, CommandTy::RespOkNodata);

            // FIXME once we support resizing we also need to check that the current and target size match
            if self.displays[display_id].active_resource != Some(framebuffer.id) {
                let scanout_request = Dma::new(SetScanout::new(
                    display_id as u32,
                    framebuffer.id,
                    GpuRect::new(0, 0, framebuffer.width, framebuffer.height),
                ))
                .unwrap();
                let header = self.send_request(scanout_request).await.unwrap();
                assert_eq!(header.ty, CommandTy::RespOkNodata);
                self.displays[display_id].active_resource = Some(framebuffer.id);
            }

            let flush = ResourceFlush::new(
                framebuffer.id,
                damage.clip(framebuffer.width, framebuffer.height).into(),
            );
            let header = self.send_request(Dma::new(flush).unwrap()).await.unwrap();
            assert_eq!(header.ty, CommandTy::RespOkNodata);
        });
    }

    fn hw_cursor_size(&self) -> Option<(u32, u32)> {
        Some((64, 64))
    }

    fn handle_cursor(&mut self, cursor: Option<&CursorPlane<Self::Buffer>>, dirty_fb: bool) {
        if let Some(cursor) = cursor {
            if dirty_fb {
                self.update_cursor(
                    &cursor.framebuffer,
                    cursor.x,
                    cursor.y,
                    cursor.hot_x,
                    cursor.hot_y,
                );
            } else {
                self.move_cursor(cursor.x, cursor.y);
            }
        } else {
            if dirty_fb {
                self.disable_cursor();
            }
        }
    }
}

pub struct GpuScheme {}

impl<'a> GpuScheme {
    pub fn new(
        config: &'a mut GpuConfig,
        control_queue: Arc<Queue<'a>>,
        cursor_queue: Arc<Queue<'a>>,
        transport: Arc<dyn Transport>,
        has_edid: bool,
    ) -> Result<GraphicsScheme<VirtGpuAdapter<'a>>, Error> {
        let adapter = VirtGpuAdapter {
            config,
            control_queue,
            cursor_queue,
            transport,
            has_edid,
            displays: vec![],
            hidden_cursor: None,
        };

        Ok(GraphicsScheme::new(
            adapter,
            "display.virtio-gpu".to_owned(),
            false,
        ))
    }
}
