/*
 * TextTileSet.java
 *
 * immutable
 *
 * rename: HexTileSet
 *
 * Created on 19 November 2004, 07:28
 */

package net.pengo.hexdraw.layout;

import java.awt.Font;
import java.awt.FontMetrics;
import java.awt.Graphics;
import net.pengo.splash.FontMetricsCache;

/**
 *
 * @author  Que
 */
public class TextTileSet extends TileSet {
    private String font;
    private String alphabet;
    private boolean antialias;
    private int bitsPerTile = -1; //lazy
    
    public TextTileSet(String font, boolean antialias) {
        this(font, "0123456789abcdef", antialias);
    }

    /* alphabet is "0123456789abcdef" */
    public TextTileSet(String font, String alphabet, boolean antialias) {
        this.font = font;
        this.alphabet = alphabet;
        this.antialias = antialias;
    }

    public boolean isMonospaced() {
        return true;
    }
    
    public boolean isHMonospaced() {
        return true;
    }
    
    // for text: yes (eg LF, CR, TAB, Byte order Mark)
    public boolean hasControlCodes() {
        return false;
    }
    
    public int getBitsPerTile() {
        
        // calculate minimum number of bits required for this many tiles
        if (bitsPerTile == -1) {
            // 2^bits = getNumTiles()
            bitsPerTile = (int) Math.ceil( Math.log(getNumTiles()) / Math.log(2) ); // = 4
        }
            
        //System.out.println("bitsPerTile=" + bitsPerTile + " Math.log(2)=" + Math.log(2) + " Math.log(getNumTiles())="+ Math.log(getNumTiles()) + " getNumTiles()=" + getNumTiles());
        
        return bitsPerTile;
    }
    
    public int getNumTiles() {
        return alphabet.length();
    }
    
    public String getName() {
        return "tileset";
    }
    
    private String getFontName() {
        return font;
    }
    
    private Font getFont() {
        return FontMetricsCache.singleton().getFont( getFontName() );
    }
    
    private FontMetrics getFontMetrics() {
        return FontMetricsCache.singleton().getFontMetrics( getFontName() );
    }

    public int maxWidth() {
        FontMetrics fm = getFontMetrics();
        
        return fm.getMaxAdvance();
    }
    
    public int maxHeight() {
        FontMetrics fm = getFontMetrics();
        
        //fixme: this seems a bit crap ?
        return fm.getMaxAscent() + fm.getMaxDescent();
    }

    public void draw(Graphics g, int tile) {
        //fixme: meant to be pre-drawn
        FontMetrics fm = getFontMetrics();
        g.setFont(getFont());
        //fixme: antialias..
        
        //System.out.println("tile=" + tile);
        char x = alphabet.charAt(tile);
        g.drawString(x+"", 0, fm.getDescent());
        //g.drawString(tile+"", 0, fm.getDescent()); // temp
    }
}
