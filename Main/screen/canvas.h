#ifndef __MCU_H__
#define __MCU_H__

#ifdef __cplusplus
extern "C" {
#endif

#include "stdint.h"
#include "stdbool.h"
#include "usbd_video_conf.h"

// NV12 format (4:2:0)
// Y plane, size UVC_WIDTH x UVC_HEIGHT bytes
// UV plane, interleaved (UV UV UV ...), size UVC_WIDTH x CANVAS_HEIGHT

#define CANVAS_HEIGHT (UVC_HEIGHT / 2)
#define TEXT_ROWS (CANVAS_HEIGHT / 8)

// NV12 uncompressed for better compatibility
// for BW every row in Y plane is repeated
// for color, each pixel translates to 2x2 in UVC, that way we don't lose colors
//#define CANVAS_WIDTH (UVC_WIDTH / 2)
#define BUFFER_SIZE (UVC_WIDTH * CANVAS_HEIGHT)
#if (BUFFER_TAIL_SIZE > 0)
#define BUFFER1_SIZE (BUFFER_SIZE - BUFFER_TAIL_SIZE)
#endif

typedef union {
    struct {
        uint8_t Cb;
        uint8_t Cr;
    };
    uint16_t value;
} NV12_UV_t;

// Store only UV plane,
// since we only support 64 specific colors, Y plane can be derived
extern uint8_t canvas_buffer[];
#if (BUFFER_TAIL_SIZE > 0)
extern uint8_t canvas_tail[];
#endif

void Clear(uint8_t color);
void SetPixel(uint16_t x, uint16_t y, uint8_t color);
uint8_t GetPixel(uint16_t x, uint16_t y);
void ClearBW(uint8_t color);
void SetPixelBW(uint16_t x, uint16_t y, uint8_t color);
uint8_t GetPixelBW(uint16_t x, uint16_t y);

void FillBufferColor(uint32_t offset, uint8_t* out);
void FillBufferBW(uint32_t offset, uint8_t* out);
extern void (*FillBuffer)(uint32_t, uint8_t*);

#ifdef __cplusplus
}
#endif

#endif

