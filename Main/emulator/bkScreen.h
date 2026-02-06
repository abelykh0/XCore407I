#pragma once

#include <stdint.h>
#include "screen/screen.h"

extern uint16_t port0177664;

namespace bk
{

class BkScreen: public Display::Screen
{
public:
    BkScreen();

    // Video RAM
    int ll_byte(uint32_t addr, uint8_t* byte);
    int ll_word(uint32_t addr, uint16_t* word);
    int sl_byte(uint32_t addr, uint8_t byte);
    int sl_word(uint32_t addr, uint16_t word);

    virtual ~BkScreen() = default;
};

}
