/*
 * Created on 17/01/2005
 *
 * TODO To change the template for this generated file go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
package net.pengo.hexdraw.layout;

/**
 * @author Que
 *
 * TODO To change the template for this generated type comment go to
 * Window - Preferences - Java - Code Style - Code Templates
 */
public class AsciiTileSet extends TextTileSet {

	public AsciiTileSet(String font, boolean antialias) {
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
    	return (char)tile;
    }


}
