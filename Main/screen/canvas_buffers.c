#include "canvas.h"

#define PLANE_WIDTH (CANVAS_WIDTH * 2)
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

uint8_t canvas_buffer[BUFFER_SIZE - BUFFER_TAIL_SIZE] __attribute__((aligned(4)));
// CCMRAM is not on the AHB bus matrix, so USB_OTG_HS internal DMA cannot reach it.
// canvas_tail must only ever be read by the CPU (e.g. copy_words into a normal-RAM
// USB packet buffer), never passed directly to USBD_LL_Transmit/USBD_LL_PrepareReceive.
uint8_t canvas_tail[BUFFER_TAIL_SIZE] __attribute__((section(".ccmram"), aligned(4)));

void (*FillBuffer)(uint32_t, uint8_t*) = FillBufferColor;

//uint8_t bk_ram[16384] __attribute__((section(".ccmram"), aligned(4)));
