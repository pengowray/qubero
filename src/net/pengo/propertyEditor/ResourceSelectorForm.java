/**
 * ResourceSelectorForm.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import net.pengo.pointer.JavaPointer;

public class ResourceSelectorForm extends AbstractResourcePropertiesForm {
    
    private JavaPointer sp; //fixme: should probably allow SmartPointer
    
    public ResourceSelectorForm(JavaPointer sp) {
        super();
        this.sp = sp;
    }
    
    protected PropertyPage[] getMenu() {
        return new PropertyPage[]{
                new ResourceSelectorPage(sp, this)
        };
    }
    
    
}

