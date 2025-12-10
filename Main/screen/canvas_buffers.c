#include "canvas.h"

#define PLANE_WIDTH (CANVAS_WIDTH * 2)
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

uint8_t canvas_buffer[BUFFER_SIZE] __attribute__((aligned(4)));
