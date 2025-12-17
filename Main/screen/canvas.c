#include "string.h"
#include "screen/canvas.h"
#include "screen/copy_words.h"

#define CANVAS_WIDTH (UVC_WIDTH / 2)
#define PLANE_WIDTH UVC_WIDTH
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

extern NV12_UV_t uv_table[];
extern uint16_t y_table[];
extern uint8_t rgb_table[];

NV12_UV_t* uv_plane = (NV12_UV_t*)canvas_buffer;
#if (BUFFER_TAIL_SIZE > 0)
NV12_UV_t* uv_tail = (NV12_UV_t*)canvas_tail;
#define PART1_SIZE (BUFFER1_SIZE / sizeof(NV12_UV_t))
#define PART2_SIZE (BUFFER_TAIL_SIZE / sizeof(NV12_UV_t))
#endif

static uint8_t buffer[2][PLANE_WIDTH * 3 / 2] __attribute__((section(".ccmram"), aligned(4)));
static uint8_t* currentRow = buffer[0];
static uint8_t* nextRow = buffer[1];
static uint32_t uvRow = 0;

// PACKET_SIZE_NO_HEADER is 1.5 * UVC_WIDTH
// Every packet we fill next 2/3 of PLANE_WIDTH
// One packet is either on one or two UV rows

// ╔════════╦════════════╦═════════╦═════════════════════════════════════════════════════════════════════╦═══════════════════════╗
// ║ Packet ║   Y Row    ║  UV Row ║                      Action Sequence                                ║ Modified buffers      ║
// ╠════════╬════════════╬═════════╬═════════════════════════════════════════════════════════════════════╬═══════════════════════╣
// ║  N/A   ║            ║    0    ║ while we are processing the UV plane, prepare currentRow (0)        ║ currentRow (0) ██████ ║
// ╠════════╬════════════╬═════════╬═════════════════════════════════════════════════════════════════════╬═══════════════════════╣
// ║    0   ║ 0  ██████  ║    0    ║ 1. use currentRow (0) to fill "out"                                 ║                       ║
// ║        ║ 1  ███░░░  ║         ║ 2. prepare nextRow (1)                                              ║ nextRow    (1) ██████ ║
// ╠════════╬════════════╬═════════╬═════════════════════════════════════════════════════════════════════╬═══════════════════════╣
// ║    1   ║ 1  ░░░███  ║    0    ║ 1. use second half of currentRow (0) and nextRow (1) to fill "out"  ║                       ║
// ║        ║ 2  ██████  ║    1    ║ 2. === swap buffers === currentRow (1), nextRow (2)                 ║                       ║
// ║        ║            ║         ║ 3. prepare nextRow (2)                                              ║ nextRow    (2) ██████ ║
// ╠════════╬════════════╬═════════╬═════════════════════════════════════════════════════════════════════╬═══════════════════════╣
// ║    2   ║ 3  ██████  ║    1    ║ 1. use currentRow (1) and first half of nextRow (2) to fill "out"   ║                       ║
// ║        ║ 4  ███░░░  ║    2    ║ 2. === swap buffers === currentRow (2), nextRow (3)                 ║                       ║
// ║        ║            ║         ║ 3. prepare nextRow (3)                                              ║ nextRow    (3) ██████ ║
// ╠════════╬════════════╬═════════╬═════════════════════════════════════════════════════════════════════╬═══════════════════════╣
// ║    3   ║ 4  ░░░███  ║    2    ║ 1. use currentRow (2) to fill "out"                                 ║                       ║
// ║        ║ 5  ██████  ║         ║ 2. === swap buffers === currentRow (3), nextRow (4)                 ║ nextRow    (4) ░░░░░░ ║
// ╠════════╬════════════╬═════════╬═════════════════════════════════════════════════════════════════════╬═══════════════════════╣
// ║    4   ║ 6  ██████  ║    3    ║ Same as packet 0                                                    ║                       ║
// ║        ║ 7  ███░░░  ║         ║                                                                     ║ nextRow    (4) ██████ ║
// ╚════════╩════════════╩═════════╩═════════════════════════════════════════════════════════════════════╩═══════════════════════╝

static inline void PrepareNextRow()
{
	if (uvRow >= UVC_HEIGHT / 2)
	{
		// Should never happen
		return;
	}

	uint16_t* out_buffer = (uint16_t*)nextRow;
	uint16_t* out_buffer_end = out_buffer + CANVAS_WIDTH - 1;
    uint32_t sourceOffset = uvRow * CANVAS_WIDTH;

	for (; out_buffer <= out_buffer_end; out_buffer++)
    {
        NV12_UV_t uv_value;
#if (BUFFER_TAIL_SIZE > 0)
        if (sourceOffset < PART1_SIZE)
        {
            uv_value = uv_plane[sourceOffset];
        }
        else
        {
            uv_value = uv_tail[sourceOffset - PART1_SIZE];
        }
#else
        uv_value = uv_plane[sourceOffset];
#endif

        uint8_t hash = uv_value.Cb ^ uv_value.Cr;
        uint16_t y_values = y_table[hash];
        *out_buffer = y_values;

        sourceOffset++;
        if (sourceOffset > UVC_HEIGHT * CANVAS_WIDTH)
        {
        	// end of Y plane
        	break;
        }
    }

}

static inline void SwapBuffers()
{
	uint8_t* temp = currentRow;
	currentRow = nextRow;
	nextRow = temp;
}

void FillBufferColor(uint32_t offset, uint8_t* out)
{
	if (offset < Y_PLANE_SIZE)
    {
		// Y plane is derived from the UV plane
    	// since we only support selected 64 colors

		switch (offset / PACKET_SIZE_NO_HEADER % 4)
		{
		case 0:
			// use currentRow to fill "out"
			copy_words((const uint32_t*)currentRow, (uint32_t*)out, PLANE_WIDTH / sizeof(uint32_t));
			copy_words((const uint32_t*)currentRow, (uint32_t*)(out + PLANE_WIDTH), PLANE_WIDTH / 2 / sizeof(uint32_t));

			uvRow++;
			PrepareNextRow();

			break;

		case 1:
			// use second half of currentRow and nextRow to fill "out"
			copy_words((const uint32_t*)(currentRow + CANVAS_WIDTH), (uint32_t*)out, PLANE_WIDTH / 2 / sizeof(uint32_t));

			// TODO workaround this is only correct for 512x512 and 768 buffer size
			if (Y_PLANE_SIZE - offset == 256)
			{
				copy_words((const uint32_t*)canvas_buffer, (uint32_t*)(out + (PLANE_WIDTH / 2)), PLANE_WIDTH / sizeof(uint32_t));
				return;
			}
			copy_words((const uint32_t*)nextRow, (uint32_t*)(out + (PLANE_WIDTH / 2)), PLANE_WIDTH / sizeof(uint32_t));

			SwapBuffers();
			uvRow++;
			PrepareNextRow();

			break;

		case 2:
			// use currentRow and first half of nextRow to fill "out"
			copy_words((const uint32_t*)currentRow, (uint32_t*)out, PLANE_WIDTH / sizeof(uint32_t));
			copy_words((const uint32_t*)nextRow, (uint32_t*)(out + PLANE_WIDTH), PLANE_WIDTH / 2 / sizeof(uint32_t));

			SwapBuffers();
			uvRow++;
			PrepareNextRow();

			break;

		default:
			// use currentRow to fill "out"
			copy_words((const uint32_t*)(currentRow + CANVAS_WIDTH), (uint32_t*)out, PLANE_WIDTH / 2 / sizeof(uint32_t));
			copy_words((const uint32_t*)currentRow, (uint32_t*)(out + (PLANE_WIDTH / 2)), PLANE_WIDTH / sizeof(uint32_t));

			SwapBuffers();

			break;
		}
    }
    else
    {
    	// UV plane as stored as is

    	int32_t canvasOffset = offset - Y_PLANE_SIZE;

#if (BUFFER_TAIL_SIZE > 0)
    	int32_t wordsRemaining = PACKET_SIZE_NO_HEADER / sizeof(uint32_t);
		while (wordsRemaining > 0)
		{
			uint32_t wordsToCopy = wordsRemaining;
			if (canvasOffset < BUFFER1_SIZE)
			{
				uint32_t nextOffset = canvasOffset + (wordsToCopy * sizeof(uint32_t));
				if (nextOffset > BUFFER1_SIZE)
				{
					wordsToCopy = (BUFFER1_SIZE - canvasOffset) / sizeof(uint32_t);
				}

				copy_words((const uint32_t*)(canvas_buffer + canvasOffset), (uint32_t*)out, wordsToCopy);
			}
			else
			{
				copy_words((const uint32_t*)(canvas_tail + canvasOffset - BUFFER1_SIZE), (uint32_t*)out, wordsToCopy);
			}

			wordsRemaining -= wordsToCopy;
			canvasOffset += wordsToCopy;
		}

#else
    	copy_words((const uint32_t*)(canvas_buffer + canvasOffset), (uint32_t*)out, PACKET_SIZE_NO_HEADER / sizeof(uint32_t));
#endif

        if (canvasOffset < BUFFER_SIZE - PACKET_SIZE_NO_HEADER)
        {
        	return;
        }

        // If last packet, need to prepare next buffer
        uvRow = 0;
		PrepareNextRow();
		SwapBuffers();
    }
}

void Clear(uint8_t color)
{
    NV12_UV_t uv_value = uv_table[color & 0x3f];
    for (uint32_t i = 0; i < PART1_SIZE; i++)
    {
    	uv_plane[i] = uv_value;
    }
#if (BUFFER_TAIL_SIZE > 0)
    for (uint32_t i = 0; i < PART2_SIZE; i++)
    {
    	uv_tail[i] = uv_value;
    }
#endif
}

void SetPixel(uint16_t x, uint16_t y, uint8_t color)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return; // out of canvas
    }

    NV12_UV_t uv_value = uv_table[color & 0x3f];

#if (BUFFER_TAIL_SIZE > 0)
    uint32_t offset = y * CANVAS_WIDTH + x;
    if (offset < PART1_SIZE)
    {
    	uv_plane[offset] = uv_value;
    }
    else
    {
    	uv_tail[offset - PART1_SIZE] = uv_value;
    }
#else
    uv_plane[y * CANVAS_WIDTH + x] = uv_value;
#endif
}

uint8_t GetPixel(uint16_t x, uint16_t y)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return 0; // out of canvas
    }

    NV12_UV_t uv_value;
#if (BUFFER_TAIL_SIZE > 0)
    uint32_t offset = y * CANVAS_WIDTH + x;
    if (offset < PART1_SIZE)
    {
    	uv_value = uv_plane[offset];
    }
    else
    {
    	uv_value = uv_tail[offset - PART1_SIZE];
    }
#else
    uv_value = uv_plane[y * CANVAS_WIDTH + x];
#endif

    uint8_t hash = uv_value.Cb ^ uv_value.Cr;

    return rgb_table[hash];
}
