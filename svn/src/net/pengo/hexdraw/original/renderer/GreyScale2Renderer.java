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

import net.pengo.hexdraw.original.Place;

public class GreyScale2Renderer extends SeperatorRenderer {

    public GreyScale2Renderer(FontMetrics fm, int columnCount, boolean render) {
        super(fm,columnCount,render);
    }
    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
    	//// draw grey scale:
        FontMetrics fm = g.getFontMetrics();
        
        int charsHeight = fm.getHeight();
        int charsWidth = charsHeight;
        
        int col, col2;
        
        for ( int i=0; i < baLength; i++ ) {
            col = ~((int)ba[i]) & 0xf0; // the gamma of left/outer square
            col2 = (~((int)ba[baOffset+i]) & 0x0f) << 4; // the gamma of right/inner square
            col = col | (col >> 4); // turn a0 into aa
            col2 = col2 | (col2 >> 4);
            
            if (selecta[i])
                g.setColor(new Color(col/3*2,col/3*2,col/2+128));
            else
                g.setColor(new Color(col,col,col));
            
            g.fillRect(i *charsWidth, 0, charsWidth-1, charsHeight-1);
            
            if (selecta[i])
                g.setColor(new Color(col2/3*2,col2/3*2,col2/2+128));
            else
                g.setColor(new Color(col2,col2,col2));
            
            g.fillRect((i * charsWidth) + (charsWidth/2), (int)(charsHeight/2), (charsWidth/2)-1, (charsHeight/2)-1);
            //g.fillRect(i *charsWidth, 0, charsWidth, charsHeight);
        }
        
        //return columnCount * charsWidth;
    }
    
    public int getWidth() {
        return fm.getHeight() * columnCount;
    }    
    
    public long whereAmI(int x, int y, long lineAddress){
        //height is same as width (square pieces)
        return lineAddress + (x / fm.getHeight());
    }    

    public String toString() {
        return "Grey scale II (unsigned)";
    }

}
