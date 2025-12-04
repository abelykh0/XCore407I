#include "string.h"
#include "screen/canvas.h"

extern NV12_UV_t uv_table[];
extern uint16_t y_table[];
extern uint8_t rgb_table[];

#define UINT16_DUP(p)  ((uint16_t)(p) << 8 | (uint16_t)(p))

#define PLANE_WIDTH (CANVAS_WIDTH * 2)
#define BUFFER_SIZE (CANVAS_WIDTH * CANVAS_HEIGHT * 2)
#define Y_PLANE_SIZE (BUFFER_SIZE * 2)

uint8_t canvas_buffer[BUFFER_SIZE] /*__attribute__(( section(".sram2") ))*/;
NV12_UV_t* uv_plane = (NV12_UV_t*)canvas_buffer;

// returns true, if it is a Y plane
// PACKET_SIZE_NO_HEADER is chosen so
// one packet is either fully in Y PLANE or UV PLANE
void FillBuffer(uint32_t offset, uint8_t* out)
{
    if (offset < Y_PLANE_SIZE)
    {
		// Y plane is derived from the UV plane
    	// since we only support selected 64 colors

    	//memset(out, 0xAA, PACKET_SIZE_NO_HEADER);
    	//return;

    	uint32_t bufferOffset = (offset / PLANE_WIDTH / 2) * CANVAS_WIDTH;
        uint32_t yCol = offset % PLANE_WIDTH;
        uint16_t* out_buffer = (uint16_t*)out;

        for (uint32_t i = 0; i < 250; i += 2)
        {
            uint32_t x = (yCol + i) >> 1;

            NV12_UV_t uv_value = uv_plane[bufferOffset + x];
            uint8_t hash = uv_value.Cb ^ uv_value.Cr;
            uint16_t y_values = y_table[hash];
            *out_buffer = y_values;
            out_buffer++;


            //if (x >= CANVAS_WIDTH) break;   // end of row

            //uint8_t index = canvas_buffer[bufferOffset + x];
            //out[i] = Colors[index][0];    // Y component
            //out[i + 1] = out[i];
        }
    }
    else
    {
    	// UV plane as stored as is
        uint32_t uvOffset = offset - Y_PLANE_SIZE;
        uint32_t bufferOffset = (uvOffset / PLANE_WIDTH) * CANVAS_WIDTH;
        bufferOffset += (uvOffset % PLANE_WIDTH) >> 1;

    	memcpy(out, canvas_buffer + bufferOffset, PACKET_SIZE_NO_HEADER);
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

