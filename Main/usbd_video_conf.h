#ifndef __USBD_VIDEO_CONF__H__
#define __USBD_VIDEO_CONF__H__

#define UVC_WIDTH                   256U
#define UVC_HEIGHT                  192U
#define UVC_CAM_FPS_HS              5U

#define USBD_UVC_FORMAT_UNCOMPRESSED
#define UVC_UNCOMPRESSED_GUID       UVC_GUID_YUY2
#define UVC_BITS_PER_PIXEL          16U

#define UVC_ISO_FS_MPS              512 // do not remove, bug in composite builder?
#define UVC_ISO_HS_MPS              512 //1024
#define UVC_HS_HEADER_SIZE          2U
#define PACKET_SIZE_NO_HEADER       ((unsigned int)(UVC_ISO_HS_MPS - UVC_HS_HEADER_SIZE))
#define UVC_MAX_FRAME_SIZE          (UVC_WIDTH * UVC_HEIGHT * 2U)
#define PACKETS_IN_FRAME            (UVC_FRAME_SIZE / PACKET_SIZE_NO_HEADER)
#define UVC_IN_EP                   0x83U

#endif
