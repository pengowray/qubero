/*
 * TileSet.java
 *
 * Created on 19 November 2004, 01:02
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;

/**
 *
 * @author  Que
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
}
