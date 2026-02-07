#include "bkscreen.h"
#include <string.h>
#include "bkEmu.h"

//   0020 Screen mode 0 - 512x256, FF - 256x256
//   0021 Screen inversion 0 - off, FF - on

// 0..7 Shift
// 9 Extended memory 0 - on, 1 - off
uint16_t port0177664;

namespace bk
{

BkScreen::BkScreen()
{
	port0177664 = 01330;
}

int BkScreen::ll_byte(uint32_t addr, uint8_t* byte)
{
	if (addr < 0x4000 || addr > 0x7FFF)
	{
		return 0;
	}

	addr -= 0x4000;
	bool isNarrow = (RamBuffer[0x0020] == 0);
	uint8_t result = 0;
	int y = addr / 64;

	if (isNarrow)
	{
		// each byte is 8 BW pixels
		int x = (addr % 64) * 8;
		for (int i = 0; i < 8; i++)
		{
			uint8_t realColor = ::GetPixel(x + i, y);

			result >>= 1;
			if (realColor != 0x00)
			{
				result |= 0x80;
			}

		}
	}
	else
	{
		// each byte is 4 color pixels
		int x = (addr % 64) * 4;
		for (int i = 0; i < 4; i++)
		{
			uint8_t realColor = ::GetPixel(x + i, y);

			uint8_t color = 0x00;
			switch (realColor)
			{
			case 0x00:
				color = 0x00 << 6; // black
				break;
			case 0x30:
				color = 0x01 << 6; // blue
				break;
			case 0x0c:
				color = 0x02 << 6; // green
				break;
			case 0x03:
				color = 0x03 << 6; // red
				break;
			}

			result >>= 2;
			result |= color;
		}
	}

	*byte = result;
	return OK;
}

int BkScreen::ll_word(uint32_t addr, uint16_t* word)
{
    uint8_t lo = 0;
    uint8_t hi = 0;

    ll_byte(addr, &lo);
    ll_byte(addr + 1, &hi);
    *word = static_cast<uint16_t>(lo) | (static_cast<uint16_t>(hi) << 8);

    return OK;
}

int BkScreen::sl_byte(uint32_t addr, uint8_t byte)
{
	if (addr < 0x4000 || addr > 0x7FFF)
	{
		return OK;
	}

	addr -= 0x4000;
	bool isNarrow = (RamBuffer[0x0020] == 0);
	int y = addr / 64;

	if (isNarrow)
	{
		// each byte is 8 BW pixels
		int x = (addr % 64) * 8;
		for (int i = 0; i < 8; i++)
		{
			uint8_t color = byte & 0x01;
			uint8_t realColor = 0;
			switch (color)
			{
			case 0:
				realColor = 0x00; // black
				break;
			case 1:
				realColor = 0x3F; // white
				break;
			}

			::SetPixelBW(x + i, y, realColor);

			byte >>= 1;
		}
	}
	else
	{
		// each byte is 4 color pixels
		int x = (addr % 64) * 4;
		for (int i = 0; i < 4; i++)
		{
			uint8_t color = byte & 0x03;
			uint8_t realColor = 0;
			switch (color)
			{
			case 0:
				realColor = 0x00; // black
				break;
			case 1:
				realColor = 0x30; // blue
				break;
			case 2:
				realColor = 0x0c; // green
				break;
			case 3:
				realColor = 0x03; // red
				break;
			}

			::SetPixel(x + i, y, realColor);

			byte >>= 2;
		}
	}

	return OK;
}

int BkScreen::sl_word(uint32_t addr, uint16_t word)
{
    uint8_t* p = reinterpret_cast<uint8_t*>(&word);

    sl_byte(addr, p[0]);
    sl_byte(addr + 1, p[1]);

    return OK;
}

}
