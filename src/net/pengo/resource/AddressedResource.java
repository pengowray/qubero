/**
 * AddressedResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

public interface AddressedResource {
    //FIXME: implements qnoderesource
    
    public SelectionResource getSelectionResource();
    
    public void setSelectionResource(SelectionResource selRes);
    
}

