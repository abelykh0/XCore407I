#include "stm32F4xx_hal.h"
#include <stdio.h>
#include "rtc.h"

#include "demo_colors.h"
#include "screen/screen.h"

#define GREY_ON_BLACK  0x2A10
#define WHITE_ON_BLACK  0x3F00

static void show_colors(uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b);

static Display::Screen screen;

void init_demo_colors()
{
	screen.SetAttribute(GREY_ON_BLACK);
	screen.Clear();

	// Frame
    screen.PrintAt(0, 0, "\xC9"); // ╔
    screen.PrintAt(TEXT_COLUMNS - 1, 0, "\xBB"); // ╗
    screen.PrintAt(0, TEXT_ROWS - 1, "\xC8"); // ╚
    screen.PrintAt(TEXT_COLUMNS - 1, TEXT_ROWS - 1, "\xBC"); // ╝
    for (uint16_t i = 1; i < TEXT_COLUMNS - 1; i++)
    {
        screen.PrintAt(i, 0, "\x0CD"); // ═
        screen.PrintAt(i, TEXT_ROWS - 1, "\x0CD"); // ═
    }
    for (uint16_t i = 1; i < TEXT_ROWS - 1; i++)
    {
        screen.PrintAt(0, i, "\x0BA"); // ║
        screen.PrintAt(TEXT_COLUMNS - 1, i, "\x0BA"); // ║
    }

    // Colors
    show_colors(3, 2, 4, 4, 4);
    show_colors(10, 2, 4, 0, 0);
    show_colors(17, 2, 0, 4, 0);
    show_colors(24, 2, 0, 0, 4);

    show_colors(10, 6, 0, 4, 4);
    show_colors(17, 6, 4, 4, 0);
    show_colors(24, 6, 4, 0, 4);

    show_colors(3, 10, 0, 1, 2);
    show_colors(3, 11, 0, 1, 3);
    show_colors(3, 12, 0, 2, 1);
    show_colors(3, 13, 0, 2, 3);
    show_colors(3, 14, 0, 3, 1);
    show_colors(3, 15, 0, 3, 2);
    show_colors(3, 16, 1, 0, 2);
    show_colors(3, 17, 1, 0, 3);
    show_colors(3, 18, 1, 1, 2);
    show_colors(3, 19, 1, 1, 3);
	show_colors(3, 20, 1, 2, 0);
    show_colors(3, 21, 1, 2, 1);

    show_colors(10, 12, 1, 2, 2);
    show_colors(10, 13, 1, 2, 3);
    show_colors(10, 14, 1, 3, 0);
    show_colors(10, 15, 1, 3, 1);
    show_colors(10, 16, 1, 3, 2);
    show_colors(10, 17, 1, 3, 3);
    show_colors(10, 18, 2, 0, 1);
    show_colors(10, 19, 2, 0, 3);
    show_colors(10, 20, 2, 1, 0);
    show_colors(10, 21, 2, 1, 1);

    show_colors(17, 12, 2, 1, 2);
    show_colors(17, 13, 2, 1, 3);
    show_colors(17, 14, 2, 2, 1);
    show_colors(17, 15, 2, 2, 3);
    show_colors(17, 16, 2, 3, 0);
    show_colors(17, 17, 2, 3, 1);
    show_colors(17, 18, 2, 3, 2);
    show_colors(17, 19, 2, 3, 3);
    show_colors(17, 20, 3, 0, 1);
    show_colors(17, 21, 3, 0, 2);

    show_colors(24, 12, 3, 1, 0);
    show_colors(24, 13, 3, 1, 1);
    show_colors(24, 14, 3, 1, 2);
    show_colors(24, 15, 3, 1, 3);
    show_colors(24, 16, 3, 2, 0);
    show_colors(24, 17, 3, 2, 1);
    show_colors(24, 18, 3, 2, 2);
    show_colors(24, 19, 3, 2, 3);
    show_colors(24, 20, 3, 3, 1);
    show_colors(24, 21, 3, 3, 2);

	screen.SetAttribute(WHITE_ON_BLACK);
}

int32_t loop_demo_colors()
{
	char showTime[20];

	RTC_DateTypeDef dateStruct;
	RTC_TimeTypeDef timeStruct;
	HAL_RTC_GetTime(&hrtc, &timeStruct, RTC_FORMAT_BIN);
	HAL_RTC_GetDate(&hrtc, &dateStruct, RTC_FORMAT_BIN); // important
	sprintf(showTime, " %.2d:%.2d:%.2d ", timeStruct.Hours, timeStruct.Minutes, timeStruct.Seconds);
    screen.PrintAlignCenter(0, showTime);

	return 0;// Ps2_GetScancode();
}

// 0..3 - fixed
// 4 - variable
static void show_colors(uint16_t x, uint16_t y, uint8_t r, uint8_t g, uint8_t b)
{
	if ((r < 4) && (g < 4) && (b < 4))
	{
		uint8_t color = (b << 4) | (g << 2) | r;
    	screen.SetAttribute((color << 8));
    	screen.PrintAt(x, y, "\xDB\xDB\xDD"); // ██▌

    	char buf[20];
    	screen.SetAttribute(GREY_ON_BLACK);
    	sprintf(buf, "%03u", CONVERT_6BIT_TO_DDD(color));
    	screen.PrintAt(x + 3, y, buf);

    	return;
	}

	bool isGreyscale = (r == g && r == b && r == 4);

	for (uint8_t variable = 1; variable < 4; variable++)
	{
		uint8_t color = 0;
		if (r < 4)
		{
			color |= r;
		}
		else
		{
			color |= variable;
		}

		if (g < 4)
		{
			color |= (g << 2);
		}
		else
		{
			color |= (variable << 2);
		}

		if (b < 4)
		{
			color |= (b << 4);
		}
		else
		{
			color |= (variable << 4);
		}

		if (!isGreyscale && ((color & 0x3) * 0x15 == color))
		{
			continue;
		}

    	screen.SetAttribute((color << 8));
    	screen.PrintAt(x, y, "\xDB\xDB\xDD"); // ██▌

    	char buf[20];
    	screen.SetAttribute(GREY_ON_BLACK);
    	sprintf(buf, "%03u", CONVERT_6BIT_TO_DDD(color));
    	screen.PrintAt(x + 3, y, buf);

    	y++;
	}
}

