/*
 * Created on Jan 25, 2004
 */
package net.pengo.resource;


public class SignatureTypeResource extends Resource {
    private TypeResource result;
    private TypeResource obj;
    private TypeResource[] param;
    
    public SignatureTypeResource(
            TypeResource result,
            TypeResource obj,
            TypeResource[] param) {
        this.result = result;
        this.obj = obj;
        this.param = param;
    }
    
    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }
    
    
    /**
     * @return Returns the obj.
     */
    public TypeResource getObj() {
        return obj;
    }

    /**
     * @return Returns the param.
     */
    public TypeResource[] getParam() {
        return param;
    }

    /**
     * @return Returns the result.
     */
    public TypeResource getResult() {
        return result;
    }

}
