/**
 * ResourceFactory.java
 *
 * @author Created by Omnicore CodeGuide
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

