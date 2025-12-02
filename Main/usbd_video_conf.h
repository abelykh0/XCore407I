#ifndef __USBD_VIDEO_CONF__H__
#define __USBD_VIDEO_CONF__H__

#define UVC_WIDTH                   512
#define UVC_HEIGHT                  384
#define UVC_CAM_FPS_HS              20U

#define USBD_UVC_FORMAT_UNCOMPRESSED
//#define UVC_UNCOMPRESSED_GUID       UVC_GUID_YUY2
//#define UVC_BITS_PER_PIXEL          16U
#define UVC_UNCOMPRESSED_GUID       UVC_GUID_NV12
#define UVC_BITS_PER_PIXEL          12U

#define UVC_ISO_HS_MPS              768 // for whatever reason 1024 doesn't work
#define UVC_ISO_FS_MPS              UVC_ISO_HS_MPS // do not remove, bug in composite builder?
#define UVC_HS_HEADER_SIZE          2U
#define PACKET_SIZE_NO_HEADER       ((unsigned int)(UVC_ISO_HS_MPS - UVC_HS_HEADER_SIZE))
#define UVC_MAX_FRAME_SIZE          (UVC_WIDTH * UVC_HEIGHT * UVC_BITS_PER_PIXEL / 8)
#define PACKETS_IN_FRAME            (UVC_FRAME_SIZE / PACKET_SIZE_NO_HEADER)
#define UVC_IN_EP                   0x83U

#endif
