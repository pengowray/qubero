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

package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.math.BigInteger;

import net.pengo.hexdraw.original.Place;

public class AddressRenderer extends SeperatorRenderer {
    int base;
    private final int lengthInChar = 10;
    //FIXME: proper calc:  lengthInChar = root.getLength().toString(base) + 2; // longest "hex" address can be (as a string), plus two (for a ": ")
    
    public AddressRenderer(FontMetrics fm, int columnCount, boolean render) {
        this(fm, columnCount, render, 16);
    }
    
    public AddressRenderer(FontMetrics fm, int columnCount, boolean render, int base) {
        super(fm, columnCount, render);
        this.base = base;
    }
    
    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
        //// draw address:
        g.setColor(Color.blue);
        FontMetrics fm = g.getFontMetrics();
        String addr = BigInteger.valueOf(lineNumber).toString(base);
        //addr = new String(addr.getBytes(), 0, 8); // right justify ?
        int rightJust = getWidth() - fm.stringWidth(addr);
        g.drawString(addr, rightJust, fm.getAscent() );
        if (addr.length() > lengthInChar) {
            //FIXME: address is too long. truncate or abbrevriate
            System.out.println("address field too long");
        }
    }
    
    public int getWidth() {
        return fm.charWidth('W')*lengthInChar;
    }

    public long whereAmI(int x, int y, long lineAddress){
        return -1;
    }
    
    public String toString() {
        return "Address" + base;
    }

}
