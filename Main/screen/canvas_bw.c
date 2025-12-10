#include <string.h>
#include "screen/canvas.h"
#include "screen/copy_words.h"

#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

#ifdef BW

extern const uint8_t grayscale_table[];
static uint32_t currentRow = 0;

void FillBuffer(uint32_t offset, uint8_t* out)
{
	if (offset < Y_PLANE_SIZE)
    {
		uint32_t* copyFrom = (uint32_t*)(canvas_buffer + (CANVAS_WIDTH * currentRow));

		// every 2nd row of the Y plane is stored as is

		switch (offset / PACKET_SIZE_NO_HEADER % 2)
		{
		case 0:
			copy_words(copyFrom, (uint32_t*)out, CANVAS_WIDTH / sizeof(uint32_t));
			copy_words(copyFrom, (uint32_t*)(out + CANVAS_WIDTH), CANVAS_WIDTH / 2 / sizeof(uint32_t));

			break;

		default:
			copy_words((const uint32_t*)(copyFrom + CANVAS_WIDTH), (uint32_t*)out, CANVAS_WIDTH / 2 / sizeof(uint32_t));
			copy_words((const uint32_t*)copyFrom, (uint32_t*)(out + (CANVAS_WIDTH / 2)), CANVAS_WIDTH / sizeof(uint32_t));
			currentRow++;

			break;
		}
    }
    else
    {
    	// UV plane is constant (128, 128)
    	memset(out, 128, PACKET_SIZE_NO_HEADER);
		currentRow = 0;
    }
}

void Clear(uint8_t color)
{
	uint8_t Y = grayscale_table[color & 0x3f];
	memset(canvas_buffer, Y, BUFFER_SIZE);
}

void SetPixel(uint16_t x, uint16_t y, uint8_t color)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return; // out of canvas
    }

    canvas_buffer[y * CANVAS_WIDTH + x] = grayscale_table[color & 0x3f];
}

uint8_t GetPixel(uint16_t x, uint16_t y)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return 0; // out of canvas
    }

    return canvas_buffer[y * CANVAS_WIDTH + x];
}

#endif
