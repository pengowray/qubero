/*
 * Created on 17/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.hexdraw.layout;

import net.pengo.splash.SimplySizedFont;

/**
 * @author Que
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
