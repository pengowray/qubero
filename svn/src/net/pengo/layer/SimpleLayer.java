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
 * SimpleLayer.java
 *
 * A layer has: a mask, data, and resources
 *
 * Created on 25 August 2002, 08:31
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.layer;

import net.pengo.app.OpenFile;

abstract public class SimpleLayer {
    OpenFile openFile;
    
    // has BOUNDS == selection, start + end, or a rough box, etc.
    // has MASK, which must be within bounds, allows holes
    // BOUNDS + MASK make up the OPAQUE AREA. The OPAQUE AREA is all that user sees.
    // a SELECTION is a MASK with no RAW DATA
    
    // has RAW DATA
    // has FORUMLAE (e.g. semi-transparency)
    // ? has REACH (where the formula may draw from?)
    
    // layers may represent:
    //  - 1. a file or chunk of memory only (modify directly)
    //  - 1. a file, 2. unsaved changes.
    //  - 1. a file, 2, its memory chace, 3 undoable modifications
    //  - CVS (file over time)
    // a selection (interface a MASK)
    
    //FIXME: allow relative layer depth. e.g. "this is higher than that, and that's higher than that.."
    
    //FIXME: allow more complex bounds, eg. 2D areas. (3d areas? trees?)
    
    private int leftBound;
    private int rightBound;
    /*
    private Mask mask;
    
    public SimpleLayer(OpenFile openFile, Mask mask) {
        
    }
    */
    
    /** Creates a new instance of ChunkLayer */
    public SimpleLayer(OpenFile openFile, int left, int right, boolean opaque) {
        
    }
    
    abstract public int getLeftBound();
    abstract public int getRightBound();
    
    abstract public void insert(int offset, byte[] data);
    abstract public void delete(int offset, int count);
    abstract public void set(int offset, int count, byte[] pattern);
    
    
}
