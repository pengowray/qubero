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
import java.awt.Graphics2D;
import java.awt.Point;

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
    
    // these spacers usually layed out vertically. they dont get repeated, unless inside a repeater spacer
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
    protected void bitIsHere(LayoutCursorBuilder lc, Round round) { 
    	BitCursor bits = lc.getBits();
    	if (isHorizontal()) {
    	
	        long xOffset = 0;
	        long nextOffset = 0;
	        for (SuperSpacer s : contents) {
	            nextOffset = xOffset + s.getPixelWidth(bits);
	            if (lc.getClickX() >= xOffset && lc.getClickX() < nextOffset) {
	            	lc.getPathList().add(s);
	            	lc.addToBlinkX(xOffset);
	                s.bitIsHere(lc.perspective((int)xOffset, 0 ), round);

	                return;
	            }
	            
	            xOffset = nextOffset;
	        }
        
    	} else {

        	long yOffset = 0;
	        long nextOffset = 0;
	        for (SuperSpacer s : contents) {
	            nextOffset = yOffset +  s.getPixelHeight(bits);
	            if (lc.getClickY() >= yOffset && lc.getClickY() < nextOffset) {
	            	lc.getPathList().add(s);
	            	lc.addToBlinkY(yOffset);
	                s.bitIsHere(lc.perspective(0, (int)yOffset ), round);
	                return;
	            }
	            
	            yOffset = nextOffset;
	        }

        }
    	
    	// failure!
    	lc.setNull(true);
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
    public void paint(Graphics g, Data d, BitSegment seg, SegmentalBitSelectionModel sel, LayoutCursorDescender curs, BitSegment repaintSeg) {
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
            //FIXME: check null
            sp.paint(g, d, seg, sel, curs.descend(sp), repaintSeg);

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
	
	public String toString() {
		String r = "GroupSpacer:" + hashCode() + " horizontal:" + horizontal + " items("+ contents.length +"): "; 

		for (int i=0; i<contents.length; i++)
			r+= "[" + i + "]:" + contents[i] + ", ";
		
		return r;
	}

	public Point getBitRangeMin(LayoutCursorBuilder min) {
		if (contents.length == 0)
			return null;
		
		//FIXME: assumes elements are aligned left or aligned top. which is currently always true of course. 
		
		SuperSpacer focus = contents[0];
		
		return focus.getBitRangeMin(min);
	}


	public void updateLayoutCursor(LayoutCursorBuilder lc, LayoutCursorDescender curs) {
		//FIXME: (optimise) make a lookup table of the path items so we dont have to go thru them one at a time
		for (SuperSpacer s : contents) {
			if (s.equals(curs.pathItem())) {
				curs = curs.descend(s); //FIXME: maybe descend() should just give back boolean success?
				s.updateLayoutCursor(lc, curs);
				return;
			}
			
			if (isHorizontal()) {
				lc.addToBlinkX(s.getPixelWidth(lc.getBits()));
			} else {
				lc.addToBlinkY(s.getPixelHeight(lc.getBits()));
			}
		}
		
		lc.setNull(true);
	}

	public Point getBitRangeMax(LayoutCursorBuilder max) {
		
		//System.out.println("<group> " + max);
		if (contents.length == 0)
			return null;
		
		//FIXED: assumption may be false. really needs to go thru all contents and make a "bounding" point
		//SuperSpacer focus = contents[contents.length-1];

		//FIXME cache stuff / optimise
		//FIXME maybe should be using that iterator
		
		BitCursor bits = max.getBits();
		Point maxPoint = null;
		SuperSpacer prevSpacer = null;
		LayoutCursorBuilder perspective = max;
		
		for (SuperSpacer s : contents) {
			if (prevSpacer != null) {
				if (isHorizontal()) {
					long x = prevSpacer.getPixelWidth(bits);
					perspective = perspective.perspective((int)x, 0);
					perspective.addToBlinkX(x);
				} else {
					long y = prevSpacer.getPixelHeight(bits);
					perspective = perspective.perspective(0, (int)y);
					perspective.addToBlinkY(y);
				}
			}
			
			LayoutCursorBuilder trial = perspective.clone();
			//System.out.println(" <perspective>" + perspective);
			//System.out.println(" <trial>" + trial);

			Point p = s.getBitRangeMax(trial);
			//System.out.println(" </trial>" + trial);

			
			if (maxPoint == null) {
				maxPoint =  p;
				
			} else {
				if (p.x > maxPoint.x) {
					maxPoint = new Point(p.x, maxPoint.y);
				}

				if (p.y > maxPoint.y) {
					maxPoint = new Point(maxPoint.x, p.y);
				}
				
			}
			prevSpacer = s;
		}
		
		LayoutCursorBuilder fin = max.restorePerspective();
		fin.setBlinkX(maxPoint.x);
		fin.setBlinkY(maxPoint.y);
		
		return new Point((int)fin.getBlinkX(), (int)fin.getBlinkY());
	}
}
