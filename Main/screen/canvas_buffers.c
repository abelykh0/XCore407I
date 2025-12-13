#include "canvas.h"

#define PLANE_WIDTH (CANVAS_WIDTH * 2)
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

uint8_t canvas_buffer[BUFFER_SIZE - BUFFER_TAIL_SIZE] __attribute__((aligned(4)));
uint8_t canvas_tail[BUFFER_TAIL_SIZE] __attribute__((section(".ccmram"), aligned(4)));

void (*FillBuffer)(uint32_t, uint8_t*) = FillBufferColor;

//uint8_t bk_ram[16384] __attribute__((section(".ccmram"), aligned(4)));
