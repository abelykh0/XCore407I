#ifndef __USBD_VIDEO_CONF__H__
#define __USBD_VIDEO_CONF__H__

// 512x384 NV12 @ 20 fps

#define UVC_WIDTH                   512
#define UVC_HEIGHT                  384
#define UVC_CAM_FPS_HS              20U

#define USBD_UVC_FORMAT_UNCOMPRESSED
//#define UVC_UNCOMPRESSED_GUID       UVC_GUID_YUY2
//#define UVC_BITS_PER_PIXEL          16U
#define UVC_UNCOMPRESSED_GUID       UVC_GUID_NV12
#define UVC_BITS_PER_PIXEL          12U

#define UVC_HS_HEADER_SIZE          2U // could be up to 12

// this is chosen so when filling USB buffers, each packet is either on Y or UV plane
#define UVC_ISO_HS_MPS              (512 + UVC_HS_HEADER_SIZE)

#define UVC_ISO_FS_MPS              UVC_ISO_HS_MPS // do not remove, bug in composite builder?
#define PACKET_SIZE_NO_HEADER       (UVC_ISO_HS_MPS - UVC_HS_HEADER_SIZE)
#define UVC_MAX_FRAME_SIZE          (UVC_WIDTH * UVC_HEIGHT * UVC_BITS_PER_PIXEL / 8)
#define UVC_IN_EP                   0x83U // because CDC is using 0x81 and 0x82

#endif
