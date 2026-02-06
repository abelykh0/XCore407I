#ifndef __USBD_VIDEO_CONF__H__
#define __USBD_VIDEO_CONF__H__

#define UVC_WIDTH                   512
#define UVC_HEIGHT                  512
#define UVC_CAM_FPS_HS              12
#define BUFFER_TAIL_SIZE            19200

// Experimental fps (color):
//  320x240: 33.3 fps // 160x120 pixels // 37.5 kB
//  480x320: 25.0 fps // 240x160 pixels // 75 kB
//  512x384: 20.8 fps // 256x192 pixels // 96 kB
//  640x372: 21.3 fps // 320x186 pixels // 116.25 kB

// Experimental fps (grayscale):
//  320x240: 70.6 fps // 320x120 pixels // 37.5 kB
//  480x320: 35.3 fps // 480x160 pixels // 75 kB
//  512x384: 27.2 fps // 512x192 pixels // 96 kB
//  640x372: 22.5 fps // 640x186 pixels // 116.25 kB

#define USBD_UVC_FORMAT_UNCOMPRESSED
#define UVC_UNCOMPRESSED_GUID       UVC_GUID_NV12
#define UVC_BITS_PER_PIXEL          12U

#define UVC_HS_HEADER_SIZE          2U // could be up to 12
#define ALIGN_OFFSET                (UVC_HS_HEADER_SIZE % sizeof(uint32_t))

//#define PACKET_SIZE_NO_HEADER_BW    1020
#define PACKET_SIZE_NO_HEADER       (UVC_WIDTH * 3 / 2)

#define UVC_ISO_HS_MPS              (UVC_HS_HEADER_SIZE + PACKET_SIZE_NO_HEADER)
#define UVC_MAX_FRAME_SIZE          (UVC_WIDTH * UVC_HEIGHT * UVC_BITS_PER_PIXEL / 8)
#define UVC_ISO_FS_MPS              UVC_ISO_HS_MPS // do not remove, bug in composite builder?

#endif
