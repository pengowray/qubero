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

/*
 * TileSet.java
 *
 * Created on 19 November 2004, 01:02
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;

import net.pengo.splash.SimpleSize;

/**
 *
 * @author Peter Halasz
 */
public abstract class TileSet {
    
    /** Creates a new instance of TileSet */
    public TileSet() {
    }
    
    public abstract boolean isMonospaced();
    
    public abstract boolean isVMonospaced();
    
    // for text: yes (eg LF, CR, TAB)
    public abstract boolean hasControlCodes();
    
    public abstract int getBitsPerTile();
    
    public abstract int getNumTiles();
    
    public abstract String getName();
    
    public abstract int maxWidth();
    public abstract int getWidth(int tile);
    
    public abstract int maxHeight();
    
    public abstract void draw(Graphics g, int tile);
    
    public abstract void setSimpleSize(SimpleSize s);
    
}
