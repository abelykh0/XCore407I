#include "string.h"
#include "screen/canvas.h"

#define PHYS_WIDTH  (UVC_WIDTH * 2)
#define PHYS_HEIGHT (UVC_HEIGHT * 2)

#define Y_SIZE      (PHYS_WIDTH * PHYS_HEIGHT)
#define UV_SIZE     (UVC_WIDTH * UVC_HEIGHT * 2)
#define NV12_SIZE   (Y_SIZE + UV_SIZE)

// 2 bytes per UV pair
typedef struct __attribute__((packed)) {
    uint8_t U;
    uint8_t V;
} UVPair;

// NV12 frame
typedef struct __attribute__((packed)) {
    uint8_t Y[Y_SIZE];                      // luma
    UVPair UV[UVC_WIDTH * UVC_HEIGHT];      // UV
} CanvasNV12;

uint8_t canvas_buffer[12] /*__attribute__(( section(".sram2") ))*/;
static CanvasNV12* canvas = (CanvasNV12*)canvas_buffer;

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

void Clear(uint8_t color)
{
	uint8_t* yCbCr = Colors[color];
    uint8_t Yc = yCbCr[0];
    uint8_t Uc = yCbCr[1];
    uint8_t Vc = yCbCr[2];

    memset(canvas->Y, Yc, Y_SIZE);

    // Fill UV plane: each pixel is 2 bytes (U+V)
    for (int i = 0; i < UVC_WIDTH * UVC_HEIGHT; i++)
    {
        canvas->UV[i].U = Uc;
        canvas->UV[i].V = Vc;
    }
}

void SetPixel(uint16_t x, uint16_t y, uint8_t color)
{
    if (x >= UVC_WIDTH || y >= UVC_HEIGHT)
    {
        return; // out of canvas
    }

	uint8_t* yCbCr = Colors[color];
    uint8_t Yc = yCbCr[0];
    uint8_t Uc = yCbCr[1];
    uint8_t Vc = yCbCr[2];

    int px = x * 2; // physical coordinates
    int py = y * 2;

    // Set 2x2 block in Y plane
    for (int dy = 0; dy < 2; dy++)
    {
        for (int dx = 0; dx < 2; dx++)
        {
            canvas->Y[(py + dy) * PHYS_WIDTH + (px + dx)] = Yc;
        }
    }

    // Set UV for this logical pixel
    canvas->UV[y * UVC_WIDTH + x].U = Uc;
    canvas->UV[y * UVC_WIDTH + x].V = Vc;
}

/*
uint8_t GetPixel(uint16_t x, uint16_t y)
{
    if (x >= UVC_WIDTH || y >= UVC_HEIGHT)
    {
        return 0; // out of canvas
    }

    uint8_t Yc = canvas.Y[(y*2) * PHYS_WIDTH + (x*2)]; // pick top-left of 2x2 block
    uint8_t Uc = canvas.UV[y * UVC_WIDTH + x].U;
    uint8_t Vc = canvas.UV[y * UVC_WIDTH + x].V;

    // TODO - convert to RGB
    return Yc | Uc  | Vc;
}
*/
