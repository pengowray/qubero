/*
 * MonoSpacer.java
 *
 * rename: simple spacer?
 *
 * Created on 18 November 2004, 22:56
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Point;
import java.awt.Rectangle;
import java.io.IOException;
import net.pengo.data.Data;

/**
 *
 * @author  Que
 */
public class MonoSpacer extends Spacer {
    private TileSet tileSet;
    
    public MonoSpacer() {
       
    }
    
    public void setFontName(String fontname) {
        super.setFontName(fontname);
        tileSet = new TextTileSet(fontname,"0123456789ABCDEF",false);
    }
    
    public boolean isHMonospaced() {
        return true;
    };

    public boolean isVMonospaced() {
        return true;
    }

    public TileSet getTileSet() {
        return tileSet;
    }

    public void setTileSet(TileSet tileSet) {
        this.tileSet = tileSet;
    }
    
     
    public Point offsetCoordinates(long offset) {
        int ic = getItemCount();
        int width = tileSet.maxWidth();
        int height = tileSet.maxHeight();
        
        //top left
        return new Point( (int) (offset%ic)*width, (int) (offset/ic)*height );
    }
    
    /** Which symbol appears at this location?  */
    public long whatIsHere(int x, int y) {
        int ic = getItemCount();
        int width = tileSet.maxWidth();
        int height = tileSet.maxHeight();

        return ((long)y/height + (long)x/width);
    }
    
    public int getWidth() {
        //fixme: special case when data has less than ItemCount bytes in it
        System.out.println("tilewidth=" + tileSet.maxWidth() + " itemcount=" + getItemCount());
        return tileSet.maxWidth() * getItemCount();
    }
    
    public int getHeight() {
        System.out.println("height=" + (int) ( ( getLength() / getItemCount() ) * tileSet.maxHeight() ));
        return (int) ( ( getLength() / getItemCount() ) * tileSet.maxHeight() ) ;
    }
    
    public long getLength() {
        return getData().getLength( tileSet.getBitsPerTile() );
    }
    
    public boolean isLinear() {
        return true;
    }

    public void paintComponent(Graphics g)  {
        try {
            //super.paintComponent(g);
            Graphics2D g2 = (Graphics2D)g;
            //super.paintComponent(g2);

            long first;
            long last;
            Rectangle clipRect = g2.getClipBounds();
            System.out.println("clipRect=" + clipRect);

            if (clipRect != null)  {
                first = whatIsHere(clipRect.x, clipRect.y);

                //fixme: should find nearest, if it goes beyond
                last = whatIsHere(clipRect.x + clipRect.width, clipRect.y + clipRect.height);
                Data d = getData();

                if (last >= getLength()) {
                    last = getLength() -1;
                }

                int[] tiles = getData().readIntArray(tileSet.getBitsPerTile(), first, 0, (int)(last-first)); // (last-first+1) ?
                for (long i=first; i<last; i++) {
                    Point p = offsetCoordinates(i);
                    // very simple.. fixme: move by the right amount each time!
                    g.translate( p.x, p.y );
                    tileSet.draw(g, tiles[(int)i-(int)first]);
                    g.translate( -p.x, -p.y );
                    System.out.println("tile[" + i + "]=" + tiles[(int)i-(int)first]);
                }
            }
            else  {
                //Paint the entire component.
                first = 0;
                last = 3; // fixme!!!!!!!!!!!

                System.out.println("Warning: Whole Spacer component got painted! that shouldn't really happen.");
            }

        } catch (IOException e) {
            //fixme
            e.printStackTrace();
        }
    }
}
