/**
 * AddressedResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;
import net.pengo.selection.SimpleLongListSelectionModel;



public interface AddressedResource {
    
    public SelectionResource getSelectionResource();
    
    public void setSelectionResource(SelectionResource selRes);
}

