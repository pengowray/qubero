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

/**
 * ResourceFactory.java
 *
 * @author  Peter Halasz
 */

package net.pengo.resource;

import java.util.Collection;
import java.util.List;
import net.pengo.app.OpenFile;
import net.pengo.selection.LongListSelectionModel;

//xxx: should just be an interface.

public class OpenFileResourceFactory extends ResourceFactory {
    
    private OpenFile openFile;
    
    public OpenFileResourceFactory (OpenFile openFile) {
        this.openFile = openFile;
    }
    
    public Resource wrap(Object o) {
        if (o instanceof OpenFile)
            return wrap((OpenFile)o);
        
        if (o instanceof Resource)
            return wrap((Resource)o);
        
        if (o instanceof LongListSelectionModel)
            return wrap((LongListSelectionModel)o);

	if (o instanceof Collection)
            return wrap((Collection)o);
        
        //if (o instanceof Selection)
        //   return wrap
        
        return new ContainerResource(o);
    }
    
    // return a live selection resource for the OpenFile
    public Resource wrap(OpenFile of) {
        new Exception().printStackTrace();
        //if (of == openFile)
        return new LiveSelectionResource(openFile);
        
        //return null;
    }
    
    public Resource wrap(Collection o) {
        return new CollectionResource(o);
    }
    
    // dont wrap (already wrapped) resources
    public Resource wrap(Resource o) {
        //fixme: maybe check that o belongs to openFile ?
        return o;
    }
    
    public Resource wrap(LongListSelectionModel o) {
        System.out.println("Wrapping a selection");
        new Exception().printStackTrace();
        return new DefaultSelectionResource(openFile, o);
    }
    

//    public static TreeResource wrap(List c, OpenFile openFile) {
//        
//    }

}

