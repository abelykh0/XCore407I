#include "bkscreen.h"
#include <string.h>
#include "bkemu.h"

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

}
