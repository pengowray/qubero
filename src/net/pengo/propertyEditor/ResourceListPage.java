/**
 * AddressListPage.java
 *
 * a list of existing resources to select from for a resource
 *
 * this is crap. doesn't work anywasy. forget it.
 */

package net.pengo.propertyEditor;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.ResourceRegistry;

//fixme: shithouse for non-modal dialogs

public class ResourceListPage extends MethodSelectionPage
{
    private JavaPointer jp;
    
    private static Object[] dodgyNullPointerFix; //
    
    public ResourceListPage(PropertiesForm form, JavaPointer jp) {
        super(form, null, selectionStatic(jp), jp+"");
        this.jp = jp;
    }
    
    static private PropertyPage[] staticPages() {
        //todo
        return null;
    }
    
    static private Object[] selectionStatic(JavaPointer jp) {
        //fixme:
        //System.out.println("jp: " + jp);
        //new Error("Debug").printStackTrace();
        //System.out.println("---");
        if (jp==null)
            return dodgyNullPointerFix;
        
        //Object[] o = ResourceRegistry.instance().getAllOfType(jp.getType()).toArray();
        Object[] o = ResourceRegistry.instance().getAll().toArray();
        dodgyNullPointerFix = o;
        
        return o;
    }
    
    private Object[] selection() {
        return selectionStatic(jp);
    }
    
    public PropertyPage getSelected() {
        
        return new TextOnlyPage("Selection", selection()[selected]+"");
    }
}

