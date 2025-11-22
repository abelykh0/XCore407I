#ifndef __MCU_H__
#define __MCU_H__

#ifdef __cplusplus
extern "C" {
#endif

#include "stdint.h"
#include "usbd_video_conf.h"

extern uint8_t canvas_buffer[UVC_FRAME_SIZE];

#define TEXT_COLUMNS (UVC_WIDTH / 8)
#define TEXT_ROWS (UVC_HEIGHT / 8)

void Clear(uint8_t color);
void SetPixel(uint16_t x, uint16_t y, uint8_t color);
//uint8_t GetPixel(uint16_t x, uint16_t y);

#ifdef __cplusplus
}
#endif

#endif
