/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/*
 * PageSpacer.java
 *
 * all spacers except the TopSpacer will have fixed max sizes
 *
 * Created on 8 December 2004, 09:27
 */

package net.pengo.hexdraw.layout;

import java.awt.Graphics;
import java.awt.Point;
import java.util.Iterator;

import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;
import net.pengo.bitSelection.SegmentalBitSelectionModel;
import net.pengo.data.Data;
import net.pengo.splash.SimpleSize;


/**
 *
 * @author Peter Halasz
 */
public class GroupSpacer extends MultiSpacer {
    
    // these spacers layed out vertically. they dont get repeated, unless inside a repeater spacer
    private SuperSpacer[] contents;
    private BitCursor length; //fixme: should this be auto from max length of contents?
    private boolean horizontal; //  layout contents horizontal or vertically 
    
    /** Creates a new instance of PageSpacer */
    
    public GroupSpacer() {
    }
    
    public void setContents(SuperSpacer[] contents) {
    	this.contents = contents;
    }
    
    public long getSubSpacerCount() {
    	return contents.length;
    }
    
    //public abstract long getDeepSubSpacerCount();
    
    public long getPixelWidth(BitCursor bits) {
    	System.out.println("bits:" + bits);
    	if (bits.equals(BitCursor.zero))
    		return 0;    	
    	
        //fixme: cache result
        //fixme: messy
        
        if (horizontal) {
            // get total
            long total = 0;

            for (SuperSpacer s : contents) {
                total += s.getPixelWidth(bits);
            }

            return total;
            
        } else {
            // get max
            long max = 0;
            for (SuperSpacer s : contents) {
                if (s.getPixelWidth(bits) > max)
                    max = s.getPixelWidth(bits);
            }

            return max;
        }
    }
    
    public long getPixelHeight(BitCursor bits) {
    	if (bits.equals(BitCursor.zero))
    		return 0;    	
    	
        //if (bits.equals(getBitCount(bits))) {
            //return cached value
        //}

        if (!horizontal) {
            // get total
            long total = 0;

            for (SuperSpacer s : contents) {
                total += s.getPixelHeight(bits);
            }

            return total;
            
        } else {
            // get max
            long max = 0;
            for (SuperSpacer s : contents) {
                if (s.getPixelHeight(bits) > max)
                    max = s.getPixelHeight(bits);
            }

            return max;
        }
    }
    
    /* number of bits taken by one iteration of the contents */
    //public BitCursor getSubBitCount() {
        //return ;
    //}
    
    // AKA max bit count
    public BitCursor getBitCount(BitCursor bits) {
    	return getLength();
    	//FIXME: return getLength(bits);
    }
    
    //public long subIsHere(int x, int y, Round round);
    
    //also need a wordIsHere and lineIsHere
    //x and y are relative to this component. i.e. 0,0 is top left of this.
    public BitCursor bitIsHere(long x, long y, Round round, BitCursor bits) {
        //fixme: cache locations in tree
    	
    	if (isHorizontal()) {
    	
	        long xOffset = 0;
	        long nextOffset = 0;
	        for (SuperSpacer s : contents) {
	            nextOffset = xOffset + s.getPixelWidth(bits);
	            if (x >= xOffset && x < nextOffset) {
	                return s.bitIsHere(x - xOffset, y, round, bits);
	            }
	            
	            xOffset = nextOffset;
	        }
        
    	} else {

        	long yOffset = 0;
	        long nextOffset = 0;
	        for (SuperSpacer s : contents) {
	            nextOffset =yOffset +  s.getPixelHeight(bits);
	            if (y >= yOffset && y < nextOffset) {
	                return s.bitIsHere(x, y - yOffset, round, bits);
	            }
	            
	            yOffset = nextOffset;
	        }

        }
        
        return null; //fixme: error.. should have been in here?
    }
    
    public SpacerIterator iterator() {
        return new SpacerIterator() {
            long pos = -1;

            int xChange = 0;
            int yChange = 0;
//            int totalXChange = 0;
//            int totalYChange = 0;
            
            Point next = null;
            
            public boolean hasNext(BitCursor bits) {
                if (bits.equals(new BitCursor())) // fixme: optimise
                	return false;
            	
                if (pos < contents.length-1 )
                    return true;
                
                return false;
            }
            
            public void jumpToNext(long nextPos, BitCursor bits) {
                //fixme: nyi
                System.out.println("jumpToNext nyi");
            }
            
            public SuperSpacer next(BitCursor bits) {
                
                if (pos >= 0) {
                    if (horizontal) {
                        xChange = (int)contents[(int)pos].getPixelWidth(bits); // if horizontal spacer
//                        totalXChange += xChange;
                    } else {
                        yChange = (int)contents[(int)pos].getPixelHeight(bits);
//                        totalYChange += yChange;
                    }
                    
                    next = null;
                }
                pos++;
                return contents[(int)pos];
            }
            
            public void remove() {
                //fixme nyi. throw exception?
            }

            public SuperSpacer currentSpacer() {
                return contents[(int)pos];
            }
            
            public long currentIndex() {
                return pos;
            }
            
            public Point currentStartPoint() {
                return new Point(xChange, yChange);
            }
            
            public Point nextStartPoint(BitCursor bits) {
                if (next == null) {
                    if (horizontal) {
                        next = new Point(xChange + (int)contents[(int)pos+1].getPixelWidth(bits), yChange); //fixme: warning long->int
                    } else {
                        next = new Point(xChange, yChange + (int)contents[(int)pos+1].getPixelHeight(bits)); //fixme: warning long->int
                    }
                }
                
                return next;
            }
            
            public int getXChange() {
                return xChange;
            }
            public int getYChange() {
                return yChange;
            }
//            public int getTotalXChange() {
//                return totalXChange;
//            }
//            public int getTotalYChange() {
//                return totalYChange;
//            }

			public BitCursor getBitOffset() {
				//fixme?
				return new BitCursor();
			}

			public void init(BitSegment range, long startPos) {
				// TODO / FIXME 
			}
            
        };
    }
    
    //public abstract Point whereGoes(long sub);
    //public abstract Point whereGoes(BitCursor bit);
    
    // 0,0 is top left
    public void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel) {
        BitCursor len = seg.getLength();
        int totalXChange = 0;
        int totalYChange = 0;
        
        // check for empty
        if (len.equals(new BitCursor())) // fixme: optimise
        	return;
        
        //System.out.println(this.getClass().getName() + " start printing: " + seg);

        SpacerIterator i = iterator();
        while (i.hasNext(len)) {
            SuperSpacer sp = i.next(len);
            
            //System.out.println("  " + sp + " seg:" + seg);
            
            g.translate(i.getXChange(), i.getYChange());
            totalXChange += i.getXChange();
            totalYChange += i.getYChange();
            
            //if (seg.getLength().compareTo())
            sp.paint(g, d, seg, sel);
        }
        
        g.translate(-totalXChange, -totalYChange);
    }
    
	public BitCursor getLength() {
		if (length == null) {
			//System.out.println("error: length not set on GroupSpacer!");
			new Exception("error: length not set on GroupSpacer!").printStackTrace();
		}
		
		return length;
	}
	public void setLength(BitCursor length) {
		this.length = length;
	}
	public boolean isHorizontal() {
		return horizontal;
	}
	public void setHorizontal(boolean horizontal) {
		this.horizontal = horizontal;
	}
	
	public void setSimpleSize(SimpleSize s) {
		for (SuperSpacer c : contents) {
			c.setSimpleSize(s);
		}
	}
}
