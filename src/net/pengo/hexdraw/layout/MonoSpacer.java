/*
 * MonoSpacer.java
 *
 * rename: simple spacer?
 *
 * Created on 18 November 2004, 22:56
 */

package net.pengo.hexdraw.layout;

import java.awt.Color;
import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Point;
import java.awt.Rectangle;
import java.io.IOException;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.data.Data;

/**
 *
 * @author  Que
 */
public class MonoSpacer extends Spacer {
    private TileSet tileSet;
    private long[] bitCountArray;
    
    public MonoSpacer() {
       setItemCount(16);
       SpacerMouseListener mouse = new SpacerMouseListener(this);
       addMouseListener(mouse);
       addMouseMotionListener(mouse);
    }
    
    public void setFontName(String fontname) {
        super.setFontName(fontname);
        setItemCount(16);
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
    
    public void setBitCount(int bitCount) {
        super.setBitCount(bitCount);
        calcBitCountArray();
        
    }
    
    private void calcBitCountArray() {
        // maybe should store these as BitCursors
        bitCountArray = new long[5];
        bitCountArray[0] = tileSet.getBitsPerTile(); // letter
        bitCountArray[1] = getBitCount(); // word
        bitCountArray[2] = getItemCount() * getBitCount(); // line
        bitCountArray[3] = bitCountArray[2]; // paragraph
        bitCountArray[4] = getData().getLength()*8; // really should do this only when needed
    }
    
    /* get length in resolution */
    private long getLength(Resolution r) {
        if (r == Resolution.all)
            return 1;
        
        return getData().getLength() * getBitCount(r) / 8;
    }

    // size of unit at x resolution, in bits
    public long getBitCount(Resolution r) {
        
        return bitCountArray[r.ordinal()];
        /*
        if (r == Resolution.letter)
            return 4;
        
        if (r == Resolution.word)
            return 8;
       
        if (r == Resolution.line)
            return getBitCount(Resolution.letter) * getItemCount(Resolution.letter);
        
        
        //FIXME: URGENT: BROKEN XXX
        new Exception("bad res!").printStackTrace();
        return 99; 
         */
    }
    
    protected int getTileWidth(Resolution r) {
        int baseWidth = getTileSet().maxWidth();
        if (r == Resolution.letter)
            return baseWidth;
        
        if (r == Resolution.word)
            return baseWidth * 2;
        
        //FIXME: URGENT: BROKEN XXX
        new Exception("bad res!").printStackTrace();
        return 99;
    }
    protected int getTileHeight() {
        return getTileSet().maxHeight();
    }
    
    protected int getItemCount(Resolution r) {
        int baseCount = getItemCount();
        if (r == Resolution.letter)
            return baseCount;
        
        if (r == Resolution.word)
            return baseCount * 2;
        
        //FIXME: URGENT: BROKEN XXX
        new Exception("bad res!").printStackTrace();
        return 99;
    }

    
    public Point offsetCoordinates(BitCursor offset, Resolution res) {
        int ic = getItemCount(res);
        int width = getTileWidth(res);
        int height = getTileHeight();
        long bitCount = getBitCount(res);
        long lineBitCount = getBitCount(Resolution.line);
        
        long symbolOffset = offset.toBits() / getBitCount(res);
        
        //return new Point( (int) (% *width, (int) (offset.toBits()/ic)*height );
        return new Point( (int)(symbolOffset % getItemCount(res)) * width, (int)(symbolOffset / getItemCount(res)) * height);
    }
    
    public Point offsetCoordinates(long offset, Resolution res) {
        int ic = getItemCount(res);
        int width = getTileWidth(res);
        int height = getTileHeight();
        
        //top left
        return new Point( (int) (offset%ic)*width, (int) (offset/ic)*height );
    }
    
    /** Which symbol appears at this location?  */
    public long whatSymbolIsHere(int x, int y) {
        int ic = getItemCount();
        int width = tileSet.maxWidth();
        int height = tileSet.maxHeight();

        return ((long)y/height * ic) + (long)(x/width);
    }
    
    //remove
    public BitCursor symbolToBitCursor(long symbol) {
        //fixme: really dodgy!
        return BitCursor.newFromBits(symbol * getBitCount());
    }
    

    /*
    public BitCursor whatSymbolIsHere(int x, int y, Resolution r, boolean cursorAfter) {
        int ic = getItemCount();
        int width = tileSet.maxWidth();
        int height = tileSet.maxHeight();

        return ((long)y/height * ic) + (long)(x/width);
    }    
     */
    
    
    public BitCursor whereIsCursor(int x, int y, Resolution r) {
        return whereIsCursor(x, y, r, Round.nearest);
    }
    
    /** use Round.before and after for the bit bounds of a symbol. Use Round.round for where cursor will go on a click. */
    public BitCursor whereIsCursor(int x, int y, Resolution r, Round round) {
        System.out.println("resolution: " + r);
        
        //fixme: all pretty dodge
        //System.out.println("large resolution: " + r);
        int ic = getItemCount(r); // per row
        long bitCount = getBitCount(r);
        int width = getTileWidth(r);
        int height = getTileHeight();

        long offset = ((long)(y/height) * ic);
        if (round==Round.before) {
            offset +=  x/width;
        } else if (round==Round.after) {
            offset += Math.ceil( (float)x/width );
        } else {
            offset += Math.round( (float)x/width );
        }
        BitCursor bc = BitCursor.newFromBits(offset * bitCount);
        System.out.println("return: " + bc);
        return bc;

    }

    public Dimension getPreferredSize()  {
        return new Dimension(getWidth(), getHeight()); //FIXME: precision loss!
    }
    
    public Dimension getMinimumSize()  {
        return getPreferredSize();
    }
    
    
    public Dimension getMaximumSize()  {
        return getPreferredSize();
    }

    public int getWidth() {
        //fixme: special case when data has less than ItemCount bytes in it
        //System.out.println("tilewidth=" + tileSet.maxWidth() + " itemcount=" + getItemCount());
        return tileSet.maxWidth() * getItemCount();
    }
    
    public int getHeight() {
        //System.out.println("height=" + (int) ( ( getLength() / getItemCount() ) * tileSet.maxHeight() ));
        return (int) ( ( getLength(Resolution.letter) / getItemCount(Resolution.letter) ) * tileSet.maxHeight() ) ;
    }
    
    
    public boolean isLinear() {
        return true;
    }
    public void paintComponent(Graphics g)  {
        super.paintComponent(g);
        try {
            //super.paintComponent(g);
            Graphics2D g2 = (Graphics2D)g;
            //super.paintComponent(g2);

            BitCursor first;
            BitCursor last;
            Resolution res = Resolution.letter;
            Rectangle clipRect = g2.getClipBounds();
            //System.out.println("clipRect=" + clipRect);

            if (clipRect != null)  {
                first = whereIsCursor(clipRect.x, clipRect.y, res, Round.before);

                //fixme: should find nearest, if it goes beyond
                last = whereIsCursor(clipRect.x, clipRect.y, res, Round.after);
                int len = (int)new BitSegment(first, last).getLength().toBits();
                //last = whatSymbolIsHere(clipRect.x + clipRect.width, clipRect.y + clipRect.height);
                Data d = getData();
                //long symbolOffset = getSymbolOffset(); // no
                
                for (BitCursor cursor = first; cursor.compareTo(last) < 0; cursor = cursor.addBits(len)) {
                    BitSegment tileSeg = new BitSegment(cursor, cursor.addBits(len));
                    int tileBits = d.readBitsToInt(tileSeg);
                    
                    Point p = offsetCoordinates(cursor, res);
                    
                    //fixme: isSelectedIndex() should accept a bitsegment
                    if (getActiveFile().getActive().getSelection().isSelectedIndex(cursor)) {
                        g.setColor(Color.red);
                    } else {
                        g.setColor(Color.black);
                    }                    
                    
                    g.translate( p.x, p.y );
                    tileSet.draw(g, tileBits);
                    g.translate( -p.x, -p.y );
                    
                }
                
                //fixme: check if last if further than last .. ahould be done by whatSymbolIshere() above
                //if (last >= getBitLength()) {
                //    last = getLength() -1;
                //}
/*
                //System.out.println("clip=" + clipRect + " first=" + first + " last=" + last);
                int[] tiles = getData().readIntArray(tileSet.getBitsPerTile(), first, 0, (int)(last-first)); // (last-first+1) ?
                //for (long i=first; i<last; i++) {
                for (long i=0; i<tiles.length; i++) {
                    
                    Point p = offsetCoordinates(first + i);
                    g.translate( p.x, p.y );
                    
                    // very simple.. fixme: move by the right amount each time!
                    BitCursor loc = symbolToBitCursor(first+i);
                    if (getActiveFile().getActive().getSelection().isSelectedIndex(loc)) {
                        g.setColor(Color.red);
                    } else {
                        g.setColor(Color.black);
                    }
                    
                    tileSet.draw(g, tiles[(int)i]);
                    g.translate( -p.x, -p.y );
                    //System.out.println("tile[" + i + "]=" + tiles[(int)i-(int)first]);
                }
 */
            }
            else  {
                //Paint the entire component.
                //first = 0;
                //last = 3; // fixme!!!!!!!!!!!

                System.out.println("Warning: Whole Spacer component got painted! that shouldn't really happen.");
            }

        } catch (IOException e) {
            //fixme
            e.printStackTrace();
        }
    }

    
}
