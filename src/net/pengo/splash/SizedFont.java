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
 * @author Que
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
