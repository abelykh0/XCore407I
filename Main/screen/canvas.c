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

static uint8_t buffer[2][PACKET_SIZE_NO_HEADER];
static uint8_t* currentBuffer = buffer[0];
static uint8_t* nextBuffer = buffer[1];
static uint32_t bufferOffset = PACKET_SIZE_NO_HEADER / 2;
static NV12_UV_t* nextSourceOffset = (NV12_UV_t*)canvas_buffer;

// Returns remaining bytes
static inline uint32_t ProcessYPlane(uint32_t offset, uint8_t* out)
{
	// Y plane is derived from the UV plane
	// since we only support selected 64 colors

	int32_t bytesToCopy = (Y_PLANE_SIZE - 1) - offset;
	if (bytesToCopy <= 0)
	{
		// Not on Y Plane
		int32_t remainingBytes = (Y_PLANE_SIZE + BUFFER_SIZE - 1) - offset;
		if (remainingBytes < PACKET_SIZE_NO_HEADER)
		{
			// Very last packet may be smaller than PACKET_SIZE_NO_HEADER
			return remainingBytes;
		}

		return PACKET_SIZE_NO_HEADER;
	}

	if (bytesToCopy > PACKET_SIZE_NO_HEADER)
	{
		bytesToCopy = PACKET_SIZE_NO_HEADER;
	}

    // Ensure aligned
    copy_words((const uint32_t*)currentBuffer, (uint32_t*)out, bytesToCopy / sizeof(uint32_t));
	//memcpy(out, currentBuffer + bufferOffset, bytesToCopy);

    bufferOffset = bufferOffset == 0 ? (PACKET_SIZE_NO_HEADER / 2) : 0;
	nextSourceOffset += (PACKET_SIZE_NO_HEADER / 2);

	// Only non-zero if same packet crosses plane boundary
	return PACKET_SIZE_NO_HEADER - bytesToCopy;
}

// True if no need to process buffers yet
static inline bool ProcessUVPlane(uint32_t offset, uint8_t* out, uint32_t size)
{
	// UV plane as stored as is

	if (size == 0)
	{
		// Not on UV plane
		return true;
	}

	int32_t uvOffset = offset - Y_PLANE_SIZE;
	if (uvOffset < 0)
	{
		uvOffset = 0;
	}
    uint32_t canvasOffset = (uvOffset / CANVAS_WIDTH) * CANVAS_WIDTH;
    canvasOffset += (uvOffset % PLANE_WIDTH) >> 1;

    // Ensure aligned
    copy_words((const uint32_t*)(canvas_buffer + canvasOffset), (uint32_t*)out, size / sizeof(uint32_t));
	//memcpy(out, canvas_buffer + bufferOffset, PACKET_SIZE_NO_HEADER);

    if (canvasOffset < BUFFER_SIZE - size)
    {
    	return true;
    }

    // If last packet, need to prepare next buffer
    nextSourceOffset = uv_plane;
    bufferOffset = 0;
	return false;
}

void FillBuffer(uint32_t offset, uint8_t* out)
{
	// Y Plane
	uint32_t remainingBytes = ProcessYPlane(offset, out);

	// UV Plane
	if (ProcessUVPlane(offset, out, remainingBytes))
	{
		return;
	}

	// Fill half of the next buffer

	memset(nextBuffer + bufferOffset, 0x77, PACKET_SIZE_NO_HEADER / 2);
/*
	uint16_t* out_buffer = (uint16_t*)(nextBuffer + bufferOffset);
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
*/
    if (bufferOffset > 0)
    {
        // Switch buffers
    	uint8_t* temp = currentBuffer;
    	currentBuffer = nextBuffer;
    	nextBuffer = temp;
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

