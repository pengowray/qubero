package net.pengo.hexdraw.original.renderer;

import java.awt.Color;
import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public class GreyScaleRenderer extends SeperatorRenderer {
    private int border;
    
    /**
     * border = size of gap between tiles
     */
    public GreyScaleRenderer(FontMetrics fm, int columnCount, boolean render, int border) {
        super(fm, columnCount, render);
        this.border = border;
    }

    public GreyScaleRenderer(FontMetrics fm, int columnCount, boolean render) {
        this(fm, columnCount, render, 0);
    }

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor) {
    	//// draw grey scale:
        FontMetrics fm = g.getFontMetrics();
        
        int charsHeight = fm.getHeight();
        int charsWidth = charsHeight;
        
        int col;
        
        for ( int i=0; i < baLength; i++ ) {
	        col = ~((int)ba[baOffset+i]) & 0xff; // the "gamma" of grey squares
	        if (selecta[i])  {
	            g.setColor(new Color(col/3*2,col/3*2,col/2+128));
	        }
	        else  {
	            g.setColor(new Color(col,col,col));
	        }
	        g.fillRect(i *charsWidth, 0, charsWidth - border, charsHeight - border);
        }
        
        //return columnCount * charsWidth;
    }

    public int getWidth() {
        return fm.getHeight() * columnCount;
    }    
    
    public long whereAmI(int x, int y, long lineAddress){
        //height is same as width (square pieces)
        return lineAddress + (x / fm.getHeight());
    }     
    
    public String toString() {
        String name = "Grey scale (unsigned)";
        if (border > 0)
            name = name + " " + border + "px border";
        
        return name;
    }

}
