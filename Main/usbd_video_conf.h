#ifndef __USBD_VIDEO_CONF__H__
#define __USBD_VIDEO_CONF__H__

#define UVC_WIDTH                   256U // 272U
#define UVC_HEIGHT                  192U // 208U
#define UVC_CAM_FPS_HS              30U
#define UVC_HS_HEADER_SIZE          12U
#define UVC_ISO_HS_MPS              3072U
#define PACKET_SIZE_NO_HEADER       ((unsigned int)(UVC_ISO_HS_MPS - UVC_HS_HEADER_SIZE))

#define UVC_FRAME_SIZE              (UVC_WIDTH * UVC_HEIGHT * 3U / 2U) // nv12
#define PACKETS_IN_FRAME            (UVC_FRAME_SIZE / PACKET_SIZE_NO_HEADER)

#endif
