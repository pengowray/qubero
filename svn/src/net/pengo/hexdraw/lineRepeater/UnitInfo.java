/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * UnitInfo.java
 *
 * @author Peter Halasz
 */

package net.pengo.hexdraw.lineRepeater;

//FIXME: make flyweight
class UnitInfo {
    static final int NORMAL = 0;
    static final int EMPTY = 1;
    static final int SELECTED = 2;
    static final int SELECTED_NOFOCUS = 3;
    static final int EDITING = 4;
    static final int EDITING_NOFOCUS = 5; // another view of this byte is being edited

    protected byte b;
    protected int state;
    
    public UnitInfo(byte b, int state) {
        this.b = b;
        this.state = state;
    }
    public byte getByte(){
        return b;
    }
    public int getState() {
        return state;
    }

    public String hexString() {
        //FIXME: cache this / lazy init / table of 256 strings?
        
        int l = ((int)b & 0xf0) >> 4;
        int r = (int)b & 0x0f;

        return ""
            + (l < 0x0a ? (char)('0'+l) : (char)('a'+l-10) )
            + (r < 0x0a ? (char)('0'+r) : (char)('a'+r-10) );
    }
}


