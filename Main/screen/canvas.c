#include "string.h"
#include "screen/canvas.h"

#define BUFFER_SIZE (CANVAS_WIDTH * CANVAS_HEIGHT)
uint8_t canvas_buffer[BUFFER_SIZE] /*__attribute__(( section(".sram2") ))*/;

// Y-Cb-Cr 64 color palette (indexed by BBGGRR)
uint8_t Colors[][3] =
{
		/*000000*/ {16, 128, 128},
		/*000001*/ {38, 115, 165},
		/*000010*/ {60, 103, 203},
		/*000011*/ {82, 90, 240},
		/*000100*/ {59, 103, 97},
		/*000101*/ {81, 91, 134},
		/*000110*/ {103, 78, 171},
		/*000111*/ {124, 66, 209},
		/*001000*/ {102, 79, 65},
		/*001001*/ {124, 66, 103},
		/*001010*/ {145, 53, 140},
		/*001011*/ {167, 41, 177},
		/*001100*/ {145, 54, 34},
		/*001101*/ {166, 41, 71},
		/*001110*/ {188, 29, 109},
		/*001111*/ {210, 16, 146},
		/*010000*/ {24, 165, 122},
		/*010001*/ {46, 153, 159},
		/*010010*/ {68, 140, 197},
		/*010011*/ {90, 128, 234},
		/*010100*/ {67, 141, 91},
		/*010101*/ {89, 128, 128},
		/*010110*/ {111, 115, 165},
		/*010111*/ {133, 103, 203},
		/*011000*/ {110, 116, 59},
		/*011001*/ {132, 103, 97},
		/*011010*/ {154, 91, 134},
		/*011011*/ {176, 78, 171},
		/*011100*/ {153, 91, 28},
		/*011101*/ {175, 79, 65},
		/*011110*/ {197, 66, 103},
		/*011111*/ {218, 53, 140},
		/*100000*/ {33, 203, 116},
		/*100001*/ {55, 190, 153},
		/*100010*/ {76, 177, 191},
		/*100011*/ {98, 165, 228},
		/*100100*/ {76, 178, 85},
		/*100101*/ {97, 165, 122},
		/*100110*/ {119, 153, 159},
		/*100111*/ {141, 140, 197},
		/*101000*/ {118, 153, 53},
		/*101001*/ {140, 141, 91},
		/*101010*/ {162, 128, 128},
		/*101011*/ {184, 115, 165},
		/*101100*/ {161, 128, 22},
		/*101101*/ {183, 116, 59},
		/*101110*/ {205, 103, 97},
		/*101111*/ {227, 91, 134},
		/*110000*/ {41, 240, 110},
		/*110001*/ {63, 227, 147},
		/*110010*/ {85, 215, 185},
		/*110011*/ {107, 202, 222},
		/*110100*/ {84, 215, 79},
		/*110101*/ {106, 203, 116},
		/*110110*/ {128, 190, 153},
		/*110111*/ {149, 177, 191},
		/*111000*/ {127, 190, 47},
		/*111001*/ {149, 178, 85},
		/*111010*/ {170, 165, 122},
		/*111011*/ {192, 153, 159},
		/*111100*/ {170, 166, 16},
		/*111101*/ {191, 153, 53},
		/*111110*/ {213, 141, 91},
		/*111111*/ {235, 128, 128}
};

// returns true, if it is a Y plane
// PACKET_SIZE_NO_HEADER is chosen so
// one packet is either fully in Y PLANE or UV PLANE
void FillBuffer(uint32_t offset, uint8_t* out)
{
    if (offset < BUFFER_SIZE)
    {
        //-------------------------
        //        Y PLANE
        //-------------------------
        uint32_t yRow = offset / CANVAS_WIDTH;
        uint32_t yCol = offset % CANVAS_WIDTH;

        for (uint32_t i = 0; i < PACKET_SIZE_NO_HEADER; i++)
        {
            uint32_t x = yCol + i;
            if (x >= CANVAS_WIDTH) break;   // end of row

            uint32_t srcX = x >> 1;
            uint32_t srcY = yRow >> 1;

            uint8_t index = canvas_buffer[srcY * CANVAS_WIDTH + srcX];
            out[i] = Colors[index][0];    // Y component
        }
    }
    else
    {
        //-------------------------
        //        UV PLANE
        //-------------------------
        uint32_t uvOffset = offset - BUFFER_SIZE;

        uint32_t uvRow = uvOffset / CANVAS_WIDTH;      // 0..HEIGHT/2-1
        uint32_t uvCol = uvOffset % CANVAS_WIDTH;      // column in UV row

        for (uint32_t i = 0; i < PACKET_SIZE_NO_HEADER; i += 2)
        {
            uint32_t x = uvCol + i;
            if (x >= CANVAS_WIDTH) break;

            uint32_t srcX = x >> 1;     // every pair shares same index
            uint32_t srcY = uvRow;

            uint8_t index = canvas_buffer[srcY * CANVAS_WIDTH + srcX];

            out[i + 0] = Colors[index][1];  // U
            out[i + 1] = Colors[index][2];  // V
        }
    }
}

void Clear(uint8_t color)
{
	memset(canvas_buffer, color & 0x3f, BUFFER_SIZE);
}

void SetPixel(uint16_t x, uint16_t y, uint8_t color)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return; // out of canvas
    }

    canvas_buffer[y * CANVAS_WIDTH + x] = (color & 0x3f); // we only use 64 colors
}

uint8_t GetPixel(uint16_t x, uint16_t y)
{
    if (x >= CANVAS_WIDTH || y >= CANVAS_HEIGHT)
    {
        return 0; // out of canvas
    }

    return canvas_buffer[y * CANVAS_WIDTH + x];
}
