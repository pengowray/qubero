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
 * @author Que
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
