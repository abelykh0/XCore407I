#include <screen/screen.h>
#include "usbd_video_conf.h"
#include <string.h>

namespace Display
{

Screen::Screen()
{
	this->_isCursorVisible = false;
	this->_textRows = TEXT_ROWS;
	this->SetMode(true);
}

void Screen::Clear()
{
	if (this->_isColorMode)
	{
		::Clear(this->_attribute);
	}
	else
	{
		::ClearBW(this->_attribute);
	}
}

void Screen::SetMode(bool isColor)
{
	this->_isColorMode = isColor;
	if (this->_isColorMode)
	{
		this->_textColumns = UVC_WIDTH / 8 / 2;

		// TODO I dont really like this
		FillBuffer = FillBufferColor;
	}
	else
	{
		this->_textColumns = UVC_WIDTH / 8;

		// TODO I dont really like this
		FillBuffer = FillBufferBW;
	}
}

void Screen::SetFont(const uint8_t* font)
{
	this->_font = (uint8_t*)font;
}

void Screen::SetAttribute(uint16_t attribute)
{
	this->_attribute = attribute;
}

void Screen::SetCursorPosition(uint8_t x, uint8_t y)
{
	if (this->cursor_x == x && this->cursor_y == y)
	{
		return;
	}

	if (x >= this->_textColumns)
	{
		x = this->_textColumns - 1;
	}

	if (y >= this->_textRows)
	{
		y = this->_textRows - 1;
	}

    if (this->_isCursorVisible)
    {
    	//this->InvertColor();
    }

	this->cursor_x = x;
	this->cursor_y = y;

    if (this->_isCursorVisible)
    {
    	//this->InvertColor();
    }
}

void Screen::ShowCursor()
{
    if (!this->_isCursorVisible)
    {
    	this->_isCursorVisible = true;
    	//this->InvertColor();
    }
}

void Screen::HideCursor()
{
    if (this->_isCursorVisible)
    {
    	this->_isCursorVisible = false;
    	//this->InvertColor();
    }
}

void Screen::Print(const char* str)
{
    if (this->_font == nullptr)
    {
        return;
    }

    while (*str)
    {
        this->PrintChar(*str++, this->_attribute);
    }
}

void Screen::PrintCharacter(const char character, uint16_t attribute)
{
	this->PrintChar(character, attribute);
}

void Screen::PrintAt(uint8_t x, uint8_t y, const char* str)
{
    this->SetCursorPosition(x, y);
    this->Print(str);
}

void Screen::PrintAlignRight(uint8_t y, const char *str)
{
    uint8_t leftX = this->_textColumns - strlen(str);
    this->PrintAt(leftX, y, str);
}

void Screen::PrintAlignCenter(uint8_t y, const char *str)
{
    uint8_t leftX = (this->_textColumns - strlen(str)) / 2;
    this->PrintAt(leftX, y, str);
}

void Screen::PrintChar(char c, uint16_t color)
{
	switch (c)
	{
	case '\0': //null
		break;
	case '\n': //line feed
		if (this->cursor_y < this->_textRows - 1)
		{
			this->SetCursorPosition(0, this->cursor_y + 1);
		}
		break;
	case '\b': //backspace
		if (this->cursor_x > 0)
		{
			this->PrintCharAt(this->cursor_x - 1, this->cursor_y, ' ', color);
			this->SetCursorPosition(this->cursor_x - 1, this->cursor_y);
		}
		break;
	case 13: //carriage return
		this->cursor_x = 0;
		break;
	default:
	{
		uint8_t x = this->cursor_x;
		uint8_t y = this->cursor_y;
		this->CursorNext();
		this->PrintCharAt(x, y, c, color);
	}
		break;
	}
}

void Screen::DrawChar(const uint8_t* f, uint16_t x, uint16_t y, uint8_t c)
{
	if (c == 0)
	{
		c = 1;
	}
	uint8_t* character = (uint8_t*)f + (c * 8) - 8;

    for (int i = 0; i < 8; i++)
    {
    	uint8_t line = character[i];
        for (int j = 0; j < 8; j++)
        {
        	uint8_t color;
            if ((line << j) & 0x80)
            {
            	// foreground color
            	color = (this->_attribute >> 8) & 0x03F;
            }
            else
            {
            	// background color
            	color = this->_attribute & 0x03F;
            }

        	if (this->_isColorMode)
        	{
        		::SetPixel(x * 8 + j, y * 8 + i, color);
        	}
        	else
        	{
        		::SetPixelBW(x * 8 + j, y * 8 + i, color);
        	}
        }
    }
}

void Screen::PrintCharAt(uint8_t x, uint8_t y, unsigned char c, uint16_t color)
{
	this->_attribute = color;
	this->DrawChar(this->_font, x, y, c);
}

void Screen::PrintCharAt(uint8_t x, uint8_t y, unsigned char c)
{
	this->PrintCharAt(x, y, c, this->_attribute);
}

void Screen::CursorNext()
{
	uint8_t x = this->cursor_x;
	uint8_t y = this->cursor_y;
	if (x < this->_textColumns - 1)
	{
		x++;
	}
	else
	{
		if (y < this->_textRows - 1)
		{
			x = 0;
			y++;
		}
	}
	this->SetCursorPosition(x, y);
}

}
