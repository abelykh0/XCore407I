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

static uint8_t buffer[2][PLANE_WIDTH];
static uint8_t* currentRow = buffer[0];
static uint8_t* nextRow = buffer[1];
static uint32_t bufferOffset = PACKET_SIZE_NO_HEADER / 2;

void FillBuffer(uint32_t offset, uint8_t* out)
{
	uint32_t row;

	if (offset < Y_PLANE_SIZE)
    {
		// Y plane is derived from the UV plane
    	// since we only support selected 64 colors

    	// copy from current buffer
        // Ensure aligned
        copy_words((const uint32_t*)currentRow, (uint32_t*)out, PACKET_SIZE_NO_HEADER / sizeof(uint32_t));
    	//memcpy(out, currentBuffer + bufferOffset, PACKET_SIZE_NO_HEADER);

    	row = offset / PLANE_WIDTH + 1;
    	bufferOffset = bufferOffset == 0 ? (PACKET_SIZE_NO_HEADER / 2) : 0;
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
        row = 0;
        bufferOffset = 0;
    }

	// Fill half of the next buffer

	//memset(nextBuffer + bufferOffset, 0xFF, PACKET_SIZE_NO_HEADER / 2);

	uint16_t* out_buffer = (uint16_t*)(nextRow + bufferOffset);
	uint16_t* out_buffer_end = out_buffer + (PACKET_SIZE_NO_HEADER / 4) - 1;
    uint32_t yCol = offset % PLANE_WIDTH + (bufferOffset / 2);
    NV12_UV_t* source = uv_plane + ((row / 2) * CANVAS_WIDTH) + yCol;

	for (; out_buffer <= out_buffer_end; out_buffer++)
    {
        NV12_UV_t uv_value = *source;
        uint8_t hash = uv_value.Cb ^ uv_value.Cr;
        uint16_t y_values = y_table[hash];
        *out_buffer = y_values;

        source++;
    }

    if (bufferOffset > 0)
    {
        // Switch buffers
    	uint8_t* temp = currentRow;
    	currentRow = nextRow;
    	nextRow = temp;
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

