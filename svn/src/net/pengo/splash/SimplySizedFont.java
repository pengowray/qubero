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
 * 
 * SimplySized as in an integer from 0 to 4 or something like that.
 * 
 * @author Peter Halasz
 */
public class SimplySizedFont {
	private String fontName;
	private SimpleSize size;
	
	
	public SimplySizedFont(String fontName) {
		this(fontName, new SimpleSize());
	}

	public SimplySizedFont(String fontName, SimpleSize size) {
		super();
		this.fontName = fontName;
		this.size = size;
	}
	
	public int hashCode() {
		return (fontName + "/" + size).hashCode();
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

	public SimpleSize getSize() {
		return size;
	}
	
	public SimplySizedFont resize(SimpleSize newSize) {
		return new SimplySizedFont(fontName, newSize);
	}
	
	public Font getFont() {
		return FontMetricsCache.singleton().getFont(this);
	}
	
	public FontMetrics getFontMetrics() {
		return FontMetricsCache.singleton().getFontMetrics(this);
	}
	
	public SimplySizedFont bigger() {
		return new SimplySizedFont(fontName, size.bigger());
	}

	public SimplySizedFont smaller() {
		return new SimplySizedFont(fontName, size.smaller());
	}
}
