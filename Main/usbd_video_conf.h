#ifndef __USBD_VIDEO_CONF__H__
#define __USBD_VIDEO_CONF__H__

// 512x384 NV12 @ 24 fps
#define UVC_WIDTH                   320
#define UVC_HEIGHT                  240
#define UVC_CAM_FPS_HS              20

// Experimental fps:
//  320x240: 33.3 fps // 160x120 pixels // 37.5 kB
//  480x320: 25.0 fps // 240x160 pixels // 75 kB
//  512x384: 20.8 fps // 256x192 pixels // 96 kB
//  528x396: 20.3 fps // 264x198 pixels // 102 kB

#define USBD_UVC_FORMAT_UNCOMPRESSED
#define UVC_UNCOMPRESSED_GUID       UVC_GUID_NV12
#define UVC_BITS_PER_PIXEL          12U

#define UVC_HS_HEADER_SIZE          2U // could be up to 12
#define ALIGN_OFFSET                (UVC_HS_HEADER_SIZE % sizeof(uint32_t))

#define PACKET_SIZE_NO_HEADER       (UVC_WIDTH * 3 / 2)

#define UVC_ISO_HS_MPS              (UVC_HS_HEADER_SIZE + PACKET_SIZE_NO_HEADER)
#define UVC_ISO_FS_MPS              UVC_ISO_HS_MPS // do not remove, bug in composite builder?
#define UVC_MAX_FRAME_SIZE          (UVC_WIDTH * UVC_HEIGHT * UVC_BITS_PER_PIXEL / 8)

#endif
