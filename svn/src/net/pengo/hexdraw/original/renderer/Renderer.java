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

import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public interface Renderer {

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor);
    
    public int getWidth();
        
    public boolean isEnabled();
    public void setEnabled(boolean render);
        
    public String toString();
    
    public void addRendererListener(RendererListener l);
    public void removeRendererListener(RendererListener l);
    
    public long whereAmI(int x, int y, long lineAddress);
    
    public void setColumnCount(int cc);
    public void setFontMetrics(FontMetrics fm);
    
    public EditBox editBox();
}


/*
 * todo 
 * 
 * 1) allow proper canvase drawing size, by
 *   a) g.setClipBounds like I am doing, demonstrate shinking, it shrinks drawing range.
 * 
 */