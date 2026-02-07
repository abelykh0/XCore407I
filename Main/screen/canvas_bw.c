#include <string.h>
#include <stdbool.h>
#include "screen/canvas.h"
#include "screen/copy_words.h"

#define CANVAS_WIDTH UVC_WIDTH
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

extern const uint8_t grayscale_table[];
extern const uint8_t bw_table[];

void FillBufferBW(uint32_t offset, uint8_t* out)
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

			uint32_t wordsToCopy = (CANVAS_WIDTH - currentColumn) / sizeof(uint32_t);
			if (wordsToCopy > wordsRemaining)
			{
				wordsToCopy = wordsRemaining;
			}

			uint32_t canvasOffset = (CANVAS_WIDTH * currentRow) + currentColumn;
			uint32_t* copyFrom;

#if (BUFFER_TAIL_SIZE > 0)
			uint32_t nextOffset = canvasOffset + (wordsToCopy * sizeof(uint32_t));
			if (nextOffset >= BUFFER1_SIZE)
			{
				wordsToCopy = (nextOffset - canvasOffset) / sizeof(uint32_t);
			}

			if (canvasOffset < BUFFER1_SIZE)
		    {
				copyFrom = (uint32_t*)(canvas_buffer + canvasOffset);
		    }
		    else
		    {
				copyFrom = (uint32_t*)(canvas_tail + canvasOffset - BUFFER1_SIZE);
		    }
#else
			copyFrom = (uint32_t*)(canvas_buffer + canvasOffset);
#endif

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

void ClearBW(uint8_t color)
{
	uint8_t Y = grayscale_table[color & 0x3f];
	memset(canvas_buffer, Y, BUFFER1_SIZE);
#if (BUFFER_TAIL_SIZE > 0)
	memset(canvas_tail, Y, BUFFER_TAIL_SIZE);
#endif
}

void SetPixelBW(uint16_t x, uint16_t y, uint8_t color)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return; // out of canvas
    }

    uint32_t offset = y * CANVAS_WIDTH + x;

#if (BUFFER_TAIL_SIZE == 0)
    canvas_buffer[offset] = grayscale_table[color & 0x3f];
#else
    if (offset < BUFFER1_SIZE)
    {
        canvas_buffer[offset] = grayscale_table[color & 0x3f];
    }
    else
    {
    	canvas_tail[offset - BUFFER1_SIZE] = grayscale_table[color & 0x3f];
    }
#endif
}

uint8_t GetPixelBW(uint16_t x, uint16_t y)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return 0; // out of canvas
    }

    uint32_t offset = y * CANVAS_WIDTH + x;
    uint8_t luminosity;

#if (BUFFER_TAIL_SIZE == 0)
    luminosity = canvas_buffer[offset];
#else
    if (offset < BUFFER1_SIZE)
    {
    	luminosity = canvas_buffer[offset];
    }
    else
    {
    	luminosity = canvas_tail[offset - BUFFER1_SIZE];
    }
#endif

    return bw_table[luminosity];
}
