#ifndef __MCU_H__
#define __MCU_H__

#ifdef __cplusplus
extern "C" {
#endif

#include "stdint.h"
#include "usbd_video_conf.h"

// TODO
// - support 256x192 as 2x (512x384 YUY2)
// - STM32F407 only has 192K RAM, 2x YUY2 would not fit
// - store as 4 bits per pixel (64 indexed colors), 256x192 will need only 24 KB
// - the UVC size is 2x
// - when streaming expand each 4 bit pixel to 32 bits (YUY2)
// - repeat row 2x
// - maybe also support 4x (1024x768)

extern uint8_t canvas_buffer[UVC_MAX_FRAME_SIZE];

#define TEXT_COLUMNS (UVC_WIDTH / 8)
#define TEXT_ROWS (UVC_HEIGHT / 8)

void Clear(uint8_t color);
void SetPixel(uint16_t x, uint16_t y, uint8_t color);
//uint8_t GetPixel(uint16_t x, uint16_t y);

#ifdef __cplusplus
}
#endif

#endif
