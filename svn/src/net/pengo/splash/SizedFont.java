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
 * Created on 24/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.splash;

import java.awt.Font;
import java.awt.FontMetrics;

/**
 * @author Peter Halasz
 *
 * TODO later version should have a different concept of size than only font size
 */
public class SizedFont {
	private String fontName;
	private float size;
	
	/**
	 * @param fontName
	 * @param size
	 */
	public SizedFont(String fontName, float size) {
		super();
		this.fontName = fontName;
		this.size = size;
	}
	
	public int hashCode() {
		return (fontName + "." + size).hashCode();
	}

	public boolean equals(Object o) {
		if (o == this)
			return true;
		
		if (o.hashCode() == this.hashCode())
			return true;
		
		return false;
	}
	
	public String getFontName() {
		return fontName;
	}

	public float getSize() {
		return size;
	}
	
	public Font getFont() {
		return FontMetricsCache.singleton().getFont(this);
	}
	
	public FontMetrics getFontMetrics() {
		return FontMetricsCache.singleton().getFontMetrics(this);
	}	
}
