#include "string.h"
#include "screen/canvas.h"
#include "screen/copy_words.h"

extern NV12_UV_t uv_table[];
extern uint16_t y_table[];
extern uint8_t rgb_table[];

#define PLANE_WIDTH (CANVAS_WIDTH * 2)
#define BUFFER_SIZE (CANVAS_WIDTH * CANVAS_HEIGHT * 2)
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

uint8_t canvas_buffer[BUFFER_SIZE] __attribute__((aligned(4)));
NV12_UV_t* uv_plane = (NV12_UV_t*)canvas_buffer;

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
    NV12_UV_t* source = uv_plane + (uvRow * CANVAS_WIDTH);

	for (; out_buffer <= out_buffer_end; out_buffer++)
    {
        NV12_UV_t uv_value = *source;
        uint8_t hash = uv_value.Cb ^ uv_value.Cr;
        uint16_t y_values = y_table[hash];
        *out_buffer = y_values;

        source++;
    }
}

static inline void SwapBuffers()
{
	uint8_t* temp = currentRow;
	currentRow = nextRow;
	nextRow = temp;
}

void FillBuffer(uint32_t offset, uint8_t* out)
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

        // Ensure aligned
    	int32_t canvasOffset = offset - Y_PLANE_SIZE;
        copy_words((const uint32_t*)(canvas_buffer + canvasOffset), (uint32_t*)out, PACKET_SIZE_NO_HEADER / sizeof(uint32_t));
    	//memcpy(out, canvas_buffer + bufferOffset, PACKET_SIZE_NO_HEADER);

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
    for (uint32_t i = 0; i < CANVAS_WIDTH * CANVAS_HEIGHT; i++)
    {
    	uv_plane[i] = uv_value;
    }
}

void SetPixel(uint16_t x, uint16_t y, uint8_t color)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return; // out of canvas
    }

    NV12_UV_t uv_value = uv_table[color & 0x3f];
    uv_plane[y * CANVAS_WIDTH + x] = uv_value;
}

uint8_t GetPixel(uint16_t x, uint16_t y)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return 0; // out of canvas
    }

    NV12_UV_t uv_value = uv_plane[y * CANVAS_WIDTH + x];
    uint8_t hash = uv_value.Cb ^ uv_value.Cr;

    return rgb_table[hash];
}

