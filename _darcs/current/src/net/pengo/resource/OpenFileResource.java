/*
 * OpenFileResource.java
 *
 * Created on July 26, 2003, 2:54 PM
 */

package net.pengo.resource;

import java.util.Collection;

import net.pengo.app.OpenFile;

/**
 *
 * @author  Smiley
 */
public class OpenFileResource extends CollectionResource {
    private OpenFile openFile;
    
    /** Creates a new instance of OpenFileResource */
    public OpenFileResource(Collection resourceList, OpenFile openFile) {
        super(resourceList);
        this.openFile = openFile;
    }
    
    public void doubleClickAction() {        
        super.doubleClickAction();
        openFile.makeActive(this);
    }
    
    public String valueDesc() {
        return openFile +": "+ resourceList.size() + " item(s)";
    }    
}
