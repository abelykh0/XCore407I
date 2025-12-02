#ifndef __MCU_H__
#define __MCU_H__

#ifdef __cplusplus
extern "C" {
#endif

#include "stdint.h"
#include "stdbool.h"
#include "usbd_video_conf.h"

// NV12 format (4:2:0)
// Y plane, size CANVAS_WIDTH x CANVAS_HEIGHT bytes
// UV plane, interleaved (UV UV UV ...), size CANVAS_WIDTH x CANVAS_HEIGHT / 2 bytes

// each pixel translate to 2x2 in UVC
// that way we don't lose color when using NV12 format

#define CANVAS_WIDTH (UVC_WIDTH / 2)
#define CANVAS_HEIGHT (UVC_HEIGHT / 2)

#define TEXT_COLUMNS (CANVAS_WIDTH / 8)
#define TEXT_ROWS (CANVAS_HEIGHT / 8)

// store as 6 bits per pixel (64 indexed colors), 256x192 will need 48 KB if each pixel is stored as one byte
extern uint8_t canvas_buffer[CANVAS_WIDTH * CANVAS_HEIGHT];

void Clear(uint8_t color);
void SetPixel(uint16_t x, uint16_t y, uint8_t color);
uint8_t GetPixel(uint16_t x, uint16_t y);

void FillBuffer(uint32_t offset, uint8_t* out);

#ifdef __cplusplus
}
#endif

#endif
