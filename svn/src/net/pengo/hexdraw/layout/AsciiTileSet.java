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
 * Created on 17/01/2005
 */
package net.pengo.hexdraw.layout;

import net.pengo.splash.SimplySizedFont;

/**
 * @author Peter Halasz
 */
public class AsciiTileSet extends TextTileSet {

	boolean highbitMasked = false;
	
	public AsciiTileSet(SimplySizedFont font, boolean antialias) {
		super(font, antialias);
	}
	
    public int getBitsPerTile() {
    	return 8;
    }
    
    public int getNumTiles() {
    	return 256;
    }
    
    public String getName() {
        return "ascii";
    }    
    
    protected char tileChar(int tile) {
    	if (highbitMasked)
    		return (char)(tile & 0x8f);
			
    	return (char)tile;
    }

	public boolean isHighbitMasked() {
		return highbitMasked;
	}
	
	public void setHighbitMasked(boolean highbitMasked) {
		this.highbitMasked = highbitMasked;
	}
}
