#include <string.h>
#include <stdbool.h>
#include "screen/canvas.h"
#include "screen/copy_words.h"

#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

#ifdef BW

extern const uint8_t grayscale_table[];

void FillBuffer(uint32_t offset, uint8_t* out)
{
	if (offset < Y_PLANE_SIZE)
    {
		// every 2nd row of the Y plane is stored as is

		int32_t wordsRemaining = PACKET_SIZE_NO_HEADER / sizeof(uint32_t);
		int32_t overfowBytes = offset + PACKET_SIZE_NO_HEADER - Y_PLANE_SIZE;
		if (overfowBytes > 0)
		{
			wordsRemaining -= (overfowBytes / sizeof(uint32_t));
		}

		uint32_t* outBuffer = (uint32_t*)out;
		uint32_t packetOffset = offset;
		while (wordsRemaining > 0)
		{
			uint32_t currentRow = (packetOffset / CANVAS_WIDTH / 2);
			uint32_t currentColumn = (packetOffset % CANVAS_WIDTH);
			uint32_t* copyFrom = (uint32_t*)(canvas_buffer + (CANVAS_WIDTH * currentRow) + currentColumn);
			uint32_t wordsToCopy = (CANVAS_WIDTH - currentColumn) / sizeof(uint32_t);
			if (wordsToCopy > wordsRemaining)
			{
				wordsToCopy = wordsRemaining;
			}

			copy_words(copyFrom, outBuffer, wordsToCopy);
			outBuffer += wordsToCopy;
			wordsRemaining -= wordsToCopy;
			packetOffset += (wordsToCopy * sizeof(uint32_t));
		}

		// UV Plane
		if (overfowBytes > 0)
		{
	    	memset(outBuffer, 128, overfowBytes);
		}
    }
    else
    {
    	// UV plane is constant (128, 128)
    	memset(out, 128, PACKET_SIZE_NO_HEADER);
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
