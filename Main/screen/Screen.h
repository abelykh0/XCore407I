#ifndef _SCREEN_H
#define _SCREEN_H

#include "stdint.h"
#include "font8x8.h"
#include "canvas.h"

namespace Display
{

class Screen
{
private:
    void PrintChar(char c, uint16_t color);
    void PrintCharAt(uint8_t x, uint8_t y, unsigned char c, uint16_t color);
    void CursorNext();

protected:
    virtual void DrawChar(const uint8_t *f, uint16_t x, uint16_t y, uint8_t c);
    //virtual void InvertColor();

    bool _isCursorVisible;
    bool _isColorMode;
    uint16_t _textColumns;
    uint16_t _textRows;

    uint8_t* _font = (uint8_t*)font8x8;
    uint16_t _attribute = 0x3F01; // white on blue in color mode

public:
	Screen();
    uint8_t cursor_x = 0;
    uint8_t cursor_y = 0;

    void SetMode(bool isColor);
	void Clear();
	void SetFont(const uint8_t* font);
	void SetAttribute(uint16_t attribute);
	void SetCursorPosition(uint8_t x, uint8_t y);
	void ShowCursor();
	void HideCursor();

	void Print(const char* str);
	void PrintCharacter(const char character, uint16_t attribute);
	void PrintAt(uint8_t x, uint8_t y, const char* str);
	void PrintAlignRight(uint8_t y, const char *str);
	void PrintAlignCenter(uint8_t y, const char *str);
    void PrintCharAt(uint8_t x, uint8_t y, unsigned char c);

	virtual ~Screen() = default;
};

}

#endif
