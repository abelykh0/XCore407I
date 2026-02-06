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
	virtual ~BkScreen() = default;
};

}
