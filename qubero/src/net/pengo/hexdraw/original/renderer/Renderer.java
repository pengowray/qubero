package net.pengo.hexdraw.original.renderer;

import java.awt.FontMetrics;
import java.awt.Graphics;

import net.pengo.hexdraw.original.Place;

public interface Renderer {

    public void renderBytes( Graphics g, long lineNumber,
            byte ba[], int baOffset, int baLength, boolean selecta[],
            Place cursor);
    
    public int getWidth();
        
    public boolean isEnabled();
    public void setEnabled(boolean render);
        
    public String toString();
    
    public void addRendererListener(RendererListener l);
    public void removeRendererListener(RendererListener l);
    
    public long whereAmI(int x, int y, long lineAddress);
    
    public void setColumnCount(int cc);
    public void setFontMetrics(FontMetrics fm);
    
    public EditBox editBox();
}


/*
 * todo 
 * 
 * 1) allow proper canvase drawing size, by
 *   a) g.setClipBounds like I am doing, demonstrate shinking, it shrinks drawing range.
 * 
 */